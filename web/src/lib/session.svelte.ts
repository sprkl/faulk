import type { Engine, EngineEvents, EngineStatus } from './engine/engine'
import { probeHelper, pairHelper } from './engine/helper-client'
import { NativeEngine } from './engine/native-engine'
import { WebEngine } from './engine/web-engine'
import type { Command, MidiInput, SynthParam } from './generated/protocol'
import { defaultLang, LANG_STORAGE_KEY, type Lang, type MessageKey, translate } from './i18n/index'
import { WebMidiHost } from './midi/web-midi'
import { takePairLink } from './pair-link'
import { DEFAULT_PARAMS } from './params'


const TOKEN_KEY = 'faulk.helperToken'

type HelperState = 'checking' | 'absent' | 'incompatible' | 'available' | 'connecting' | 'connected'

function storage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}

/**
 * The page's state and the only path from the UI to an engine: every user action is a
 * `Command` passed to `run`.
 */
export class Session {
  lang = $state<Lang>(defaultLang(storage()?.getItem(LANG_STORAGE_KEY) ?? null, globalThis.navigator?.language))
  started = $state(false)
  starting = $state(false)
  status = $state<EngineStatus | null>(null)
  params = $state<Record<SynthParam, number>>({ ...DEFAULT_PARAMS })
  inputs = $state<MidiInput[]>([])
  selectedInput = $state<string | null>(null)
  midiState = $state<'unknown' | 'ok' | 'unsupported' | 'denied'>('unknown')
  activeNotes = $state<number[]>([])
  peak = $state(0)
  voices = $state(0)
  helper = $state<HelperState>('checking')
  helperProtocol = $state<number | null>(null)
  paired = $state(Boolean(storage()?.getItem(TOKEN_KEY)))
  /** Which engine plays: the page's (Web) or the helper's (Boost). */
  mode = $state<'web' | 'native'>('web')
  notice = $state<MessageKey | null>(null)
  pairingMessage = $state<{ key: MessageKey; n?: number } | null>(null)
  /** Code from a `#pair=` link, used once audio has started (browsers need a click). */
  pendingPairCode = $state<string | null>(null)

  /** MIDI input chosen in Web mode, reopened when coming back from Boost. */
  private lastWebInput: string | null = null
  private web: WebEngine | null = null
  private native: NativeEngine | null = null
  private midi: WebMidiHost | null = null

  t = (key: MessageKey, vars?: Record<string, string | number>) => translate(this.lang, key, vars)

  setLang(lang: Lang) {
    this.lang = lang
    storage()?.setItem(LANG_STORAGE_KEY, lang)
  }

  private get engine(): Engine | null {
    return this.native ?? this.web
  }

  private events(kind: 'web' | 'native'): EngineEvents {
    const current = () => this.engine?.kind === kind
    return {
      onStatus: (status) => {
        if (current()) this.status = status
      },
      onMeter: (peak, voices) => {
        if (!current()) return
        this.peak = peak
        this.voices = voices
      },
      onMidiInputs: (inputs, selected) => {
        if (kind !== 'native' || !current()) return
        this.inputs = inputs
        this.selectedInput = selected
      },
      onMidiActivity: (note, velocity) => this.markNote(note, velocity),
      onError: (code) => {
        if (code === 'session.replaced') this.notice = 'helper.replaced'
        else if (code === 'midi.connect') this.notice = 'midi.connectError'
        else if (code === 'audio.stream' && kind === 'native') {
          // The helper's output stopped (device unplugged or changed): play in the page.
          this.notice = 'audio.error'
          void this.enterWeb()
        }
      },
      onClosed: () => {
        if (kind !== 'native' || !this.native) return
        this.native = null
        this.mode = 'web'
        if (this.notice !== 'helper.replaced') this.notice = 'helper.lost'
        void this.enterWeb()
        void this.checkHelper()
      },
    }
  }

  /** Starts the Web engine and Web MIDI. Call from a click. */
  async start() {
    if (this.started || this.starting) return
    this.starting = true
    try {
      this.web = await WebEngine.create(this.events('web'))
      this.started = true
      this.notice = null
      this.replayParams(this.web)
      await this.enableWebMidi()
    } catch (error) {
      console.error(error)
      this.notice = 'start.failed'
      return
    } finally {
      this.starting = false
    }
    // Outside the try: a pairing problem is not an audio start failure.
    const code = this.pendingPairCode
    this.pendingPairCode = null
    if (code) await this.pairFromLink(code)
  }

  /** Pairs with a code from a link; a failure is a pairing failure, never thrown. */
  private async pairFromLink(code: string) {
    try {
      await this.pair(code)
    } catch (error) {
      console.error(error)
      this.pairingMessage = { key: 'helper.failed' }
    }
  }

  private async enableWebMidi() {
    if (!WebMidiHost.supported()) {
      this.midiState = 'unsupported'
      return
    }
    this.midi ??= new WebMidiHost({
      onInputs: (inputs, selected) => {
        if (this.mode !== 'web') return
        this.inputs = inputs
        this.selectedInput = selected
      },
      onCommand: (command) => {
        if (this.mode === 'web') this.run(command)
      },
      onActivity: (note, velocity) => this.markNote(note, velocity),
    })
    this.midiState = (await this.midi.enable()) ? 'ok' : 'denied'
  }

  /** The single entry point for user actions. */
  run(command: Command) {
    switch (command.type) {
      case 'synth.set_param':
        this.params[command.param] = command.value
        this.engine?.run(command)
        break
      case 'midi.list_inputs':
      case 'midi.select_input':
        if (this.native) this.native.run(command)
        else if (command.type === 'midi.select_input') {
          this.lastWebInput = command.id ?? null
          this.midi?.select(this.lastWebInput)
        } else this.midi?.refresh()
        break
      case 'note.on':
        this.engine?.run(command)
        this.markNote(command.note, command.velocity)
        break
      case 'note.off':
        this.engine?.run(command)
        this.markNote(command.note, 0)
        break
      case 'note.all_off':
        this.engine?.run(command)
        this.activeNotes = []
        break
    }
  }

  /** Records which notes sound, for the piano strip and the MIDI activity light. */
  private markNote(note: number, velocity: number) {
    const others = this.activeNotes.filter((n) => n !== note)
    this.activeNotes = velocity > 0 ? [...others, note] : others
  }

  private replayParams(engine: Engine) {
    for (const [param, value] of Object.entries(this.params) as [SynthParam, number][]) {
      engine.run({ type: 'synth.set_param', param, value })
    }
  }

  /** Probes for the helper; returns what was found. */
  async checkHelper(): Promise<HelperState> {
    this.helper = 'checking'
    const result = await probeHelper()
    if (this.native) return this.helper
    if (result.status === 'available') this.helper = 'available'
    else if (result.status === 'incompatible') {
      this.helper = 'incompatible'
      this.helperProtocol = result.protocol
    } else this.helper = 'absent'
    return this.helper
  }

  /**
   * Reads a `#pair=<code>` link from the address and clears it from the address bar.
   * A started page pairs at once; otherwise the code waits for `start`.
   */
  takePairLink(location: Location = globalThis.location, history: History = globalThis.history) {
    const code = takePairLink(location, history)
    if (code === null) return
    if (this.started) void this.pairFromLink(code)
    else this.pendingPairCode = code
  }

  async pair(code: string) {
    this.pairingMessage = null
    const result = await pairHelper(code)
    if (result.status === 'paired') {
      storage()?.setItem(TOKEN_KEY, result.token)
      this.paired = true
      await this.enterBoost()
    } else if (result.status === 'refused') {
      this.pairingMessage =
        result.attemptsLeft > 0 ? { key: 'helper.refused', n: result.attemptsLeft } : { key: 'helper.refusedNew' }
    } else {
      this.pairingMessage = { key: 'helper.failed' }
    }
  }

  /** Moves playback to the helper: silence and release the Web side, then replay the settings. */
  async enterBoost() {
    const token = storage()?.getItem(TOKEN_KEY)
    if (!token || this.native) return
    this.helper = 'connecting'
    this.notice = null
    this.web?.run({ type: 'note.all_off' })
    this.activeNotes = []
    await this.midi?.releaseAll()
    this.selectedInput = null
    try {
      const native = await NativeEngine.connect(token, this.events('native'))
      this.native = native
      this.mode = 'native'
      this.status = native.status
      this.helper = 'connected'
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
        this.paired = false
      }
    }
  }

  /** Back to the Web engine: release the helper's notes, then replay the settings. */
  async enterWeb() {
    if (this.native) {
      const native = this.native
      this.native = null
      this.mode = 'web'
      await native.close()
      this.helper = 'available'
    }
    this.activeNotes = []
    if (!this.web) return
    await this.web.resume()
    this.status = this.web.status()
    this.replayParams(this.web)
    // Reopen the input that was playing before Boost (the helper had it meanwhile).
    this.midi?.select(this.lastWebInput)
  }
}
