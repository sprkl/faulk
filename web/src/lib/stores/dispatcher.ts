import type { Engine, EngineEvents } from '../engine/engine'
import { probeHelper, pairHelper } from '../engine/helper-client'
import { peakOf } from '../engine/feedback'
import { NativeEngine } from '../engine/native-engine'
import { WebEngine } from '../engine/web-engine'
import { bringHelperCopyUpToDate } from '../engine/helper-copy'
import type { Command, ProjectEvent } from '../generated/protocol'
import { WebMidiHost } from '../midi/web-midi'
import { CoreError, coreModule, engineStep, ProjectCore } from '../project/core'
import { releaseAllNotes, requestMidiInputs } from './housekeeping'
import { takePairLink } from '../pair-link'
import { applyEffects } from './effects'
import type { EngineStore, HelperState } from './engine.svelte'
import type { ProjectStore } from './project.svelte'
import { storage, TOKEN_KEY } from './storage'

/** The epoch of the first engine stream of the page's project core (newer than a fresh engine's 0). */
const FIRST_EPOCH = 1

/**
 * The only path from the UI to an engine: every user action is a `Command` passed to `run`.
 * It owns the page's project core (the page owns the project in both modes), the engines and
 * Web MIDI, and is the only writer of the project and engine stores; the views read the
 * stores. `run` is generic: the core applies the command, the active engine gets what it
 * means for it, the project store gets the events. It also moves playback between the page
 * (Web) and the helper (Boost) by snapshot, and pairs with the helper.
 */
export class Dispatcher {
  /** Counts probes, so a late answer from an older probe never overwrites a newer one. */
  private probes = 0
  /** Code from a `#pair=` link whose pairing reached no helper, tried again on access. */
  private linkCodeToRetry: string | null = null
  /** MIDI input chosen in Web mode, reopened when coming back from Boost. */
  private lastWebInput: string | null = null
  private core: ProjectCore | null = null
  /** The engines the core has set up so far: the first takes its `Reset` in the first epoch, every later one a snapshot in a newer epoch. */
  private engineStarts = 0
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

  /** Counts Web engines, so a late event of a stopped one never reaches the one that replaced it. */
  private webGeneration = 0

  private events(kind: 'web' | 'native', generation = this.webGeneration): EngineEvents {
    // A Web engine is told apart by its generation, a helper session by the count of sessions that became current.
    const current = () =>
      this.engine?.kind === kind && generation === (kind === 'native' ? this.boosts : this.webGeneration)
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
      onMidiActivity: (note, velocity) => {
        if (kind === 'web' || current()) this.markNote(note, velocity)
      },
      onStreamRefused: (code, at) => {
        if (current()) void this.stopWeb(code, at)
      },
      onError: (code) => {
        // A helper session that was closed (or replaced by a newer one) says nothing any more.
        if (kind === 'native' && !current()) return
        if (code === 'session.replaced') this.state.notice = 'helper.replaced'
        else if (code === 'midi.connect') this.state.notice = 'midi.connectError'
        else if (code === 'audio.stream' && kind === 'native') {
          // The helper's output stopped (device unplugged or changed): play in the page.
          this.state.notice = 'audio.error'
          void this.enterWeb()
        }
      },
      onClosed: () => {
        if (kind !== 'native' || !current()) return
        this.native = null
        this.state.mode = 'web'
        if (this.state.notice !== 'helper.replaced') this.state.notice = 'helper.lost'
        void this.enterWeb()
        void this.checkHelper()
      },
    }
  }

  /**
   * Starts the Web engine and Web MIDI. Call from a click; after a stream the engine refused,
   * this is the "restart audio": a new engine starts from a snapshot of the project.
   */
  async start() {
    if (this.state.started || this.state.starting) return
    this.state.starting = true
    try {
      this.state.streamError = null
      const core = (this.core ??= ProjectCore.create(await coreModule(), 'single_synth', FIRST_EPOCH))
      const generation = ++this.webGeneration
      const web = await WebEngine.create(this.events('web', generation))
      this.web = web
      this.state.started = true
      this.state.notice = null
      this.state.status = web.status()
      this.project.show(core.project())
      if (!this.syncWeb(web)) return
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
   * Brings an engine up to the project: the first engine gets the core's setup, every later one
   * a snapshot in a newer epoch (a switch back from Boost, a restart after a refused stream).
   * False when the snapshot did not encode: the engine was stopped.
   */
  private syncWeb(web: WebEngine): boolean {
    const core = this.core!
    try {
      const { report, engine } = this.engineStarts++ === 0 ? core.setup() : core.snapshot()
      if (report.status === 'encoded') {
        web.push(engine)
        return true
      }
      void this.stopWeb(report.stream_error, 0)
    } catch (error) {
      // The core could not produce the snapshot at all: the engine has no project to play.
      console.error(error)
      void this.stopWeb(error instanceof CoreError ? error.code : 0, 0)
    }
    return false
  }

  /**
   * The Web engine stopped taking changes (the engine refused a stream, or the core could not
   * encode one): it is behind the project. Stop it and show why; the Start button, which
   * restarts audio, comes back, and the new engine starts from a snapshot.
   */
  private async stopWeb(code: number, at: number) {
    const web = this.web
    if (!web) return
    this.web = null
    this.state.streamError = { code, at }
    this.state.started = false
    this.state.status = null
    this.state.peak = 0
    this.state.voices = 0
    this.state.activeNotes = []
    await web.close().catch((error: unknown) => console.error(error))
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

  /**
   * The single entry point for user actions. The project core applies the command; the active
   * engine gets what it means for it; the project store gets the events; the page-local
   * effects (`effects.ts`) follow. A command the project refuses changes nothing.
   */
  run(command: Command) {
    const core = this.core
    // No engine (audio not started, or stopped after a refused stream): nothing to keep in step.
    if (!core || !this.engine) return
    const step = engineStep(() => core.run(command))
    if ('refused' in step) {
      // The views only send commands the project takes: a refusal is a page bug.
      console.warn('the project refused a command', command, step.refused)
      return
    }
    this.mirror(step.events)
    if ('streamError' in step) {
      // No report (it was lost): the command may have applied, read the project again.
      if (step.events.length === 0) this.refresh()
      // Applied, but its engine messages did not encode: the Web engine is behind the project.
      // The helper takes the command itself, whatever its mirror of the stream.
      if (this.native) this.native.send(command)
      else {
        console.error(`engine stream: error ${step.streamError}`)
        void this.stopWeb(step.streamError, 0)
      }
    } else this.engine?.send(command, step.push)
    applyEffects(command, {
      mode: this.state.mode,
      midi: this.midi,
      rememberWebInput: (id) => (this.lastWebInput = id),
      markNote: (note, velocity) => this.markNote(note, velocity),
      clearNotes: () => (this.state.activeNotes = []),
    })
  }

  /** Feeds the project store with the events of an applied command. */
  private mirror(events: ProjectEvent[]) {
    const core = this.core
    if (!core) return
    try {
      this.project.apply(events, () => core.project())
    } catch (error) {
      console.error(error)
    }
  }

  /** Reads the core's project again, when no report says what changed. */
  private refresh() {
    try {
      if (this.core) this.project.show(this.core.project())
    } catch (error) {
      console.error(error)
    }
  }

  /** Records which notes sound, for the piano strip and the MIDI activity light. */
  private markNote(note: number, velocity: number) {
    const others = this.state.activeNotes.filter((n) => n !== note)
    this.state.activeNotes = velocity > 0 ? [...others, note] : others
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

  /** The connection to the helper under way: a second request (button, pairing link) waits for it, so two sessions never overlap. */
  private pendingBoost: Promise<void> | null = null

  /**
   * Moves playback to the helper: connect first, so a helper that cannot be reached leaves the Web
   * engine and its held notes alone; then silence and release the Web side, and bring the helper's
   * copy to the page's settings.
   */
  async enterBoost() {
    // A failed attempt of another caller is not this caller's failure: it tries on its own.
    while (this.pendingBoost) await this.pendingBoost.catch(() => undefined)
    // Read after the wait: a pairing that finished meanwhile stored a newer token.
    const token = storage()?.getItem(TOKEN_KEY)
    if (!token || this.native) return
    const pending: Promise<void> = this.connect(token).finally(() => {
      if (this.pendingBoost === pending) this.pendingBoost = null
    })
    this.pendingBoost = pending
    await pending
  }

  private async connect(token: string) {
    this.state.helper = 'connecting'
    this.state.notice = null
    // The session as soon as it is open, and whether it became the current engine.
    const generation = this.boosts
    let opened: NativeEngine | null = null
    try {
      const session = (opened = await NativeEngine.connect(token, this.events('native', this.boosts + 1)))
      // The Web engine plays until now: it gets its notes released before the helper takes over.
      releaseAllNotes((command) => this.run(command))
      this.state.activeNotes = []
      this.native = session
      this.boosts++
      this.state.mode = 'native'
      this.state.status = session.status
      this.state.helper = 'connected'
      this.state.selectedInput = null
      // The helper gets the page's settings before anything is awaited: no note reaches it at its defaults.
      bringHelperCopyUpToDate(session, this.project.params)
      // The helper's first MIDI list may arrive before this engine became current.
      requestMidiInputs((command) => this.run(command))
      await this.midi?.releaseAll()
      // Back to Web meanwhile (the button, a lost helper): nothing left to do for this session.
      if (this.native !== session) return
      await this.web?.suspend()
    } catch {
      if (opened) {
        // The helper accepted the session and a later step failed: the switch to Web closes that
        // session, unless the helper was lost meanwhile and that switch is already done. A session
        // that never became current is closed here.
        if (this.native === opened) await this.enterWeb()
        else if (this.boosts === generation) await opened.close().catch((error: unknown) => console.error(error))
        await this.checkHelper()
        return
      }
      // The helper never became current: the Web engine still plays and needs no snapshot (one would cut a held note).
      // Forget the token only when the helper answers but refused the session (new
      // token after a reinstall); a helper that is gone or busy keeps the pairing.
      // A pairing that happened meanwhile stored a newer token: that one stays.
      if ((await this.checkHelper()) === 'available' && storage()?.getItem(TOKEN_KEY) === token) {
        storage()?.removeItem(TOKEN_KEY)
        this.state.paired = false
      }
    }
  }

  /** The switch to the Web engine under way, so a second request (button, device error, lost socket) joins it. */
  private switching: { done: Promise<void>; boosts: number } | null = null
  /** Counts the helper sessions that became current. */
  private boosts = 0

  /**
   * Back to the Web engine: catch it up by snapshot, then release the helper. The switch and the
   * snapshot come first, with no await before them: the Web engine is the current engine only once
   * it has its snapshot, so no command ever reaches it before (a snapshot's new epoch wipes what
   * came earlier, #262). Resuming the audio context and closing the helper follow.
   */
  enterWeb(): Promise<void> {
    // A helper session that began since (Boost entered again during the switch) is a new reason to switch.
    if (this.switching && this.switching.boosts === this.boosts) return this.switching.done
    const done: Promise<void> = this.switchToWeb().finally(() => {
      if (this.switching?.done === done) this.switching = null
    })
    this.switching = { done, boosts: this.boosts }
    return done
  }

  private async switchToWeb() {
    const native = this.native
    const web = this.web
    this.native = null
    this.state.mode = 'web'
    this.state.activeNotes = []
    // The project moved on while the helper played: the Web engine catches up by snapshot.
    const ready = web ? this.syncWeb(web) : false
    if (native) {
      await native.close().catch((error: unknown) => console.error(error))
      this.state.helper = 'available'
    }
    // A Web engine that was stopped (refused stream, snapshot that did not encode) is closed.
    if (!web || this.web !== web) return
    await web.resume()
    // Boost was entered again meanwhile: the helper plays, and the Web side stays as it is.
    if (this.native) return
    this.state.status = web.status()
    // Reopen the input that was playing before Boost (the helper had it meanwhile).
    if (ready) this.midi?.select(this.lastWebInput)
  }
}
