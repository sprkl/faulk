import type { Engine, EngineEvents } from '../engine/engine'
import { probeHelper, pairHelper } from '../engine/helper-client'
import { peakOf } from '../engine/feedback'
import { NativeEngine } from '../engine/native-engine'
import { WebEngine } from '../engine/web-engine'
import type { Command, SynthParam } from '../generated/protocol'
import { WebMidiHost } from '../midi/web-midi'
import { allNotesOff, setParam } from '../synth-commands'
import { takePairLink } from '../pair-link'
import type { EngineStore, HelperState } from './engine.svelte'
import type { ProjectStore } from './project.svelte'
import { storage, TOKEN_KEY } from './storage'

/**
 * The only path from the UI to an engine: every user action is a `Command` passed to `run`.
 * It owns the engines and Web MIDI, and is the only writer of the project and engine stores;
 * the views read the stores. It also moves playback between the page (Web) and the helper
 * (Boost) and pairs with the helper.
 */
export class Dispatcher {
  /** Counts probes, so a late answer from an older probe never overwrites a newer one. */
  private probes = 0
  /** Code from a `#pair=` link whose pairing reached no helper, tried again on access. */
  private linkCodeToRetry: string | null = null
  /** MIDI input chosen in Web mode, reopened when coming back from Boost. */
  private lastWebInput: string | null = null
  private web: WebEngine | null = null
  private native: NativeEngine | null = null
  private midi: WebMidiHost | null = null

  constructor(
    private readonly project: ProjectStore,
    private readonly state: EngineStore,
  ) {}

  private get engine(): Engine | null {
    return this.native ?? this.web
  }

  private events(kind: 'web' | 'native'): EngineEvents {
    const current = () => this.engine?.kind === kind
    return {
      onStatus: (status) => {
        if (current()) this.state.status = status
      },
      onFeedback: (report) => {
        if (!current()) return
        this.state.feedback = report
        this.state.peak = peakOf(report.master)
        this.state.voices = report.voices
      },
      onMidiInputs: (inputs, selected) => {
        if (kind !== 'native' || !current()) return
        this.state.inputs = inputs
        this.state.selectedInput = selected
      },
      onMidiActivity: (note, velocity) => this.markNote(note, velocity),
      onStreamRefused: (code, at) => {
        if (current()) this.state.streamError = { code, at }
      },
      onError: (code) => {
        if (code === 'session.replaced') this.state.notice = 'helper.replaced'
        else if (code === 'midi.connect') this.state.notice = 'midi.connectError'
        else if (code === 'audio.stream' && kind === 'native') {
          // The helper's output stopped (device unplugged or changed): play in the page.
          this.state.notice = 'audio.error'
          void this.enterWeb()
        }
      },
      onClosed: () => {
        if (kind !== 'native' || !this.native) return
        this.native = null
        this.state.mode = 'web'
        if (this.state.notice !== 'helper.replaced') this.state.notice = 'helper.lost'
        void this.enterWeb()
        void this.checkHelper()
      },
    }
  }

  /** Starts the Web engine and Web MIDI. Call from a click. */
  async start() {
    if (this.state.started || this.state.starting) return
    this.state.starting = true
    try {
      this.state.streamError = null
      this.web = await WebEngine.create(this.events('web'))
      this.state.started = true
      this.state.notice = null
      this.replayParams(this.web)
      await this.enableWebMidi()
    } catch (error) {
      console.error(error)
      this.state.notice = 'start.failed'
      return
    } finally {
      this.state.starting = false
    }
    // Outside the try: a pairing problem is not an audio start failure.
    const code = this.state.pendingPairCode
    this.state.pendingPairCode = null
    if (code) await this.pairFromLink(code)
  }

  /**
   * Pairs with a code from a link; a failure is a pairing failure, never thrown. A code
   * that reached no helper (the browser's local network prompt still open) is kept for
   * one more try once the user allows (`onLocalAccessGranted`).
   */
  private async pairFromLink(code: string) {
    this.linkCodeToRetry = null
    try {
      await this.pair(code)
    } catch (error) {
      console.error(error)
      this.state.pairingMessage = { key: 'helper.failed' }
    }
    if (this.state.pairingMessage?.key === 'helper.failed') this.linkCodeToRetry = code
  }

  private async enableWebMidi() {
    if (!WebMidiHost.supported()) {
      this.state.midiState = 'unsupported'
      return
    }
    this.midi ??= new WebMidiHost({
      onInputs: (inputs, selected) => {
        if (this.state.mode !== 'web') return
        this.state.inputs = inputs
        this.state.selectedInput = selected
      },
      onCommand: (command) => {
        if (this.state.mode === 'web') this.run(command)
      },
      onActivity: (note, velocity) => this.markNote(note, velocity),
    })
    this.state.midiState = (await this.midi.enable()) ? 'ok' : 'denied'
  }

  /** The single entry point for user actions. */
  run(command: Command) {
    switch (command.type) {
      case 'device.set_param':
        if (Object.hasOwn(this.project.params, command.param)) {
          this.project.params[command.param as SynthParam] = command.value
          this.engine?.run(command)
        }
        break
      case 'midi.list_inputs':
      case 'midi.select_input':
        if (this.native) this.native.run(command)
        else if (command.type === 'midi.select_input') {
          this.lastWebInput = command.id ?? null
          this.midi?.select(this.lastWebInput)
        } else this.midi?.refresh()
        break
      case 'live.note_on':
        this.engine?.run(command)
        this.markNote(command.note, command.velocity)
        break
      case 'live.note_off':
        this.engine?.run(command)
        this.markNote(command.note, 0)
        break
      case 'live.all_notes_off':
        this.engine?.run(command)
        this.state.activeNotes = []
        break
      case 'transport.play':
      case 'transport.stop':
      case 'transport.seek':
        this.engine?.run(command)
        break
    }
  }

  /** Records which notes sound, for the piano strip and the MIDI activity light. */
  private markNote(note: number, velocity: number) {
    const others = this.state.activeNotes.filter((n) => n !== note)
    this.state.activeNotes = velocity > 0 ? [...others, note] : others
  }

  private replayParams(engine: Engine) {
    for (const [param, value] of Object.entries(this.project.params) as [SynthParam, number][]) {
      engine.run(setParam(param, value))
    }
  }

  /**
   * Probes for the helper; returns what this probe found. Only the latest probe writes
   * the state: an older one that answers late (a probe that gave up behind the browser's
   * prompt) never overwrites a newer answer.
   */
  async checkHelper(): Promise<HelperState> {
    const probe = ++this.probes
    this.state.helper = 'checking'
    const result = await probeHelper()
    if (this.native) return this.state.helper
    const found: HelperState =
      result.status === 'available' ? 'available' : result.status === 'incompatible' ? 'incompatible' : 'absent'
    if (probe !== this.probes) return found
    this.state.helper = found
    if (result.status === 'incompatible') this.state.helperProtocol = result.protocol
    return found
  }

  /**
   * The browser now lets the page reach this device (`engine/local-access.ts`): probe
   * again if the last probe found nothing or is still waiting behind the prompt, then
   * retry a pairing link that could not reach the helper.
   */
  async onLocalAccessGranted() {
    if (this.native || (this.state.helper !== 'absent' && this.state.helper !== 'checking')) return
    if ((await this.checkHelper()) !== 'available') return
    const code = this.linkCodeToRetry
    this.linkCodeToRetry = null
    if (code && this.state.started && !this.state.paired && !this.native) await this.pairFromLink(code)
  }

  /**
   * Reads a `#pair=<code>` link from the address and has `clear` remove it from the
   * address bar (`pair-link.ts`).
   * A started page pairs at once; otherwise the code waits for `start`.
   */
  takePairLink(location: Pick<Location, 'hash' | 'pathname' | 'search'>, clear: (url: string) => void) {
    const code = takePairLink(location, clear)
    if (code === null) return
    if (this.state.started) void this.pairFromLink(code)
    else this.state.pendingPairCode = code
  }

  async pair(code: string) {
    this.state.pairingMessage = null
    const result = await pairHelper(code)
    if (result.status === 'paired') {
      storage()?.setItem(TOKEN_KEY, result.token)
      this.state.paired = true
      await this.enterBoost()
    } else if (result.status === 'refused') {
      this.state.pairingMessage =
        result.attemptsLeft > 0 ? { key: 'helper.refused', n: result.attemptsLeft } : { key: 'helper.refusedNew' }
    } else {
      this.state.pairingMessage = { key: 'helper.failed' }
    }
  }

  /** Moves playback to the helper: silence and release the Web side, then replay the settings. */
  async enterBoost() {
    const token = storage()?.getItem(TOKEN_KEY)
    if (!token || this.native) return
    this.state.helper = 'connecting'
    this.state.notice = null
    this.web?.run(allNotesOff())
    this.state.activeNotes = []
    await this.midi?.releaseAll()
    this.state.selectedInput = null
    try {
      const native = await NativeEngine.connect(token, this.events('native'))
      this.native = native
      this.state.mode = 'native'
      this.state.status = native.status
      this.state.helper = 'connected'
      this.replayParams(native)
      // The helper's first MIDI list may arrive before this engine became current.
      native.run({ type: 'midi.list_inputs' })
      await this.web?.suspend()
    } catch {
      this.native = null
      await this.enterWeb()
      // Forget the token only when the helper answers but refused the session (new
      // token after a reinstall); a helper that is gone or busy keeps the pairing.
      if ((await this.checkHelper()) === 'available') {
        storage()?.removeItem(TOKEN_KEY)
        this.state.paired = false
      }
    }
  }

  /** Back to the Web engine: release the helper's notes, then replay the settings. */
  async enterWeb() {
    if (this.native) {
      const native = this.native
      this.native = null
      this.state.mode = 'web'
      await native.close()
      this.state.helper = 'available'
    }
    this.state.activeNotes = []
    if (!this.web) return
    await this.web.resume()
    this.state.status = this.web.status()
    this.replayParams(this.web)
    // Reopen the input that was playing before Boost (the helper had it meanwhile).
    this.midi?.select(this.lastWebInput)
  }
}
