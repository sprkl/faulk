import type { Engine, EngineEvents } from '../engine/engine'
import { probeHelper, pairHelper } from '../engine/helper-client'
import { peakOf } from '../engine/feedback'
import { LoadMeter } from '../engine/load'
import { NativeEngine, NoOutputError } from '../engine/native-engine'
import { WebEngine } from '../engine/web-engine'
import { withFrame } from '../engine/clock'
import { bringHelperCopyUpToDate } from '../engine/helper-copy'
import type { Command, HistoryView, ProjectEvent, RunReport } from '../generated/protocol'
import { WebMidiHost } from '../midi/web-midi'
import { draftName } from '../project/draft-name'
import { CoreError, coreModule, engineStep, ProjectCore, type EngineStep, type FolderEntry, type StoredProjects } from '../project/core'
import {
  isListStored,
  isNewProject,
  isSaveCommand,
  isStorageCommand,
  isStoredProjectCommand,
  listStored as listStoredCommand,
  newProject as newProjectCommand,
  newProjectName,
} from '../project-commands'
import type { AudioExport, ExportHost } from '../export/audio-export.svelte'
import type { ProjectHost, ProjectStorage } from '../storage/storage.svelte'
import { releaseAllNotes, requestMidiInputs } from './housekeeping'
import { takePairLink } from '../pair-link'
import { applyEffects } from './effects'
import type { EngineStore, HelperState } from './engine.svelte'
import { refusalNotice, WEB_ONLY_NOTICE, type OpenOutcome, type OpenRefusal } from './open'
import type { ProjectStore } from './project.svelte'
import type { Recorder } from './recorder.svelte'
import { refusalOf } from './refusal'
import { storage, TOKEN_KEY } from './storage'

/** The epoch of the first engine stream of the page's project core (newer than a fresh engine's 0). */
const FIRST_EPOCH = 1

/** While the page waits for the helper's audio output (#525): how often it asks the helper, in milliseconds. */
export const OUTPUT_POLL_MS = 2000
/** A Boost session that loses its output sooner than this after it began counts as a flap: the next return waits longer. */
export const OUTPUT_FLAP_MS = 60_000
/** The longest the output must stay before the page goes back to Boost after flaps. */
export const OUTPUT_STEADY_MAX_MS = 60_000

/** How long a refusal stays on screen, in milliseconds. */
export const REFUSAL_TOAST_MS = 6000

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

  /** MIDI recording (#470): told of every command applied and every report of the current engine. */
  recorder: Recorder | null = null

  private get engine(): Engine | null {
    return this.native ?? this.web
  }

  /** Counts Web engines, so a late event of a stopped one never reaches the one that replaced it. */
  private webGeneration = 0

  private events(kind: 'web' | 'native', generation = this.webGeneration): EngineEvents {
    // A Web engine is told apart by its generation, a helper session by the count of sessions that became current.
    const current = () => this.engine?.kind === kind && generation === (kind === 'native' ? this.boosts : this.webGeneration)
    // Each engine's load is smoothed from its own first report (#486).
    const load = new LoadMeter()
    return {
      onStatus: (status) => {
        if (current()) this.state.status = status
      },
      onFeedback: (report) => {
        if (!current()) return
        this.state.feedback = report
        this.state.load = load.update(report.cpuLoad, report.cpuPeak, performance.now())
        this.state.peak = peakOf(report.master)
        this.state.voices = report.voices
        this.recorder?.feedback(report)
      },
      onMidiInputs: (inputs, selected) => {
        if (kind !== 'native' || !current()) return
        this.state.inputs = inputs
        this.state.selectedInput = selected
      },
      onMidiActivity: (note, velocity) => {
        if (kind === 'web' || current()) this.markNote(note, velocity)
      },
      onHelperUpdate: (update) => {
        if (kind === 'native' && current()) this.state.helperUpdate = update
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
          // The helper's output stopped (device unplugged or changed): play in the page, and go
          // back to Boost once the helper has an output again (#525). An output lost soon after
          // Boost began is a flap: the next return waits for the output to stay longer.
          const lasted = this.boostSince === null ? Infinity : Date.now() - this.boostSince
          this.outputFlaps = lasted < OUTPUT_FLAP_MS ? this.outputFlaps + 1 : 0
          this.state.notice = 'audio.error'
          // The wait starts once the switch is done, even a failed one, unless "Stay in Web" came meanwhile.
          const waits = this.outputWaits
          void this.enterWeb()
            .catch((error: unknown) => console.error(error))
            .then(() => {
              if (this.outputWaits === waits) this.awaitOutput()
            })
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
      const core = await this.ensureCore()
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
   * The name a new project gets (#473): the moment it was started, in the page's language. The stores set it
   * (`draftName` with the UI's language); a bare dispatcher names its projects in English.
   */
  nameNew: () => string = () => draftName('en', new Date())

  /** Told the new identity a copy saved as another folder took (#473): the stores carry the layout over. */
  onIdentityCopied: (id: string) => void = () => {}

  /** The page's project core, created on first use (by `start`, or by opening a project before audio starts). */
  private async ensureCore(): Promise<ProjectCore> {
    const module = await coreModule()
    return (this.core ??= ProjectCore.create(module, 'single_synth', FIRST_EPOCH, undefined, this.nameNew()))
  }

  /**
   * Opens a project folder, replacing the current project (`project.open`): `entries` are the
   * folder's files as the store read them (`FolderEntry`; `needsBytes` says which carry
   * bytes). The core replaces the project and its history and restores its session (solo, MIDI
   * input), and the engine starts over from a snapshot of the new project in a newer epoch (the
   * old project's tracks fade out in 5 ms, no note outlives it). A folder that is not a valid project of this version changes nothing and
   * says why in a notice. Works before audio starts: the first engine then starts from the opened
   * project.
   *
   * In Boost the helper cannot hold a project yet (BO-1): the page plays the opened project
   * itself, in Web mode, and says so.
   */
  async openProject(name: string, entries: FolderEntry[]): Promise<OpenOutcome> {
    const core = await this.ensureCore()
    // Nothing is touched before the folder is accepted: a refusal leaves the page as it was, input
    // device and held notes included. The old project's notes end with it (the engine starts over),
    // and the Web MIDI input is reselected below from the new session.
    try {
      core.stage(name, entries)
    } catch (error) {
      if (!(error instanceof CoreError)) throw error
      return this.refuseOpen('frame')
    }
    const step = engineStep(() => core.open(name))
    if ('refused' in step) {
      const report = step.refused
      const error = report.status === 'refused' ? report.error : null
      return this.refuseOpen(error?.code === 'folder' ? error.error.reason : 'frame')
    }
    const opened = await this.adopt(core, step)
    this.storage?.changed()
    return { status: 'opened', name: opened }
  }

  /**
   * Starts a new project (`project.new`): the default one-synth project in no folder, the history
   * emptied. Like opening, it works before audio starts. False when the core refused.
   */
  async newProject(name?: string): Promise<boolean> {
    const core = await this.ensureCore()
    const step = engineStep(() => core.run(newProjectCommand(name ?? this.nameNew())))
    if ('refused' in step) {
      console.warn('the project refused project.new', step.refused)
      this.showRefusal(step.refused)
      return false
    }
    await this.adopt(core, step)
    this.storage?.changed()
    return true
  }

  /**
   * The page takes the project that replaced its own (`project.open`, `project.new`): the
   * engine starts over from it, the notes held on the old project end, the Web MIDI input is the
   * new session's. Returns the new project's name.
   */
  private async adopt(core: ProjectCore, step: Exclude<EngineStep, { refused: unknown }>): Promise<string> {
    // Another project: the core forgot its take, the page drops its own.
    this.recorder?.engineReplaced()
    this.state.activeNotes = []
    this.state.notice = null
    this.mirror(step.events)
    if (step.events.length === 0) {
      this.refresh()
      // No report said that a project was opened: the views that hold a choice made on the old one still have to drop it (#445).
      this.project.openedRev++
    }
    const view = core.project()
    // The input of the restored session; reopened by the switch back to Web when Boost plays.
    const input = view.session.midi_input ?? null
    this.lastWebInput = input
    if (this.native) {
      // The snapshot of the switch below starts the Web engine from the opened project.
      await this.enterWeb()
      this.state.notice = WEB_ONLY_NOTICE
    } else if ('streamError' in step) {
      // Applied, but the snapshot did not encode: the Web engine, if any, is behind the project.
      if (this.web) {
        console.error(`engine stream: error ${step.streamError}`)
        void this.stopWeb(step.streamError, 0)
      }
    } else {
      this.web?.push(step.push)
      this.midi?.select(input)
    }
    return view.project.name
  }

  /** What the project's storage asks of the page's project: the controller in `lib/storage` drives it. */
  readonly host: ProjectHost = {
    dirty: () => (this.core ? this.core.dirty() : null),
    projectDirty: () => (this.core ? this.core.projectDirty() : null),
    save: (mode) => {
      if (!this.core) return null
      const frame = this.core.save(mode)
      // A copy took a new identity without an event: the mirror (and `projectId`) must follow, and so does the
      // project's layout (#473): it is the same work.
      if (mode === 'copy') {
        this.refresh()
        const id = this.project.view?.project.id
        if (id) this.onIdentityCopied(id)
      }
      return frame
    },
    saved: (applied) => this.core?.saved(applied),
    openProject: (name, entries) => this.openProject(name, entries),
    newProject: (name?: string) => this.newProject(name),
    projectId: () => this.project.view?.project.id ?? null,
    projectName: () => this.project.view?.project.name ?? null,
    packExport: async (files) => (await this.ensureCore()).packExport(files),
    packImport: async (zip) => (await this.ensureCore()).packImport(zip),
    prepare: async () => void (await this.ensureCore()),
  }

  /** The project's storage; set once by `createStores`. */
  storage: ProjectStorage | null = null

  /** The audio export (#297); set once by `createStores`. */
  audioExport: AudioExport | null = null

  /** What the audio export needs of the page's project. */
  readonly exportHost: ExportHost = {
    // The dialog shows the refusal of the export itself.
    run: (command) => this.run(command, { quiet: true }),
    songBars: () => {
      const view = this.project.view
      if (!view) return 1
      const { numerator, denominator } = view.project.meter[0]?.signature ?? { numerator: 4, denominator: 4 }
      const bar = (numerator * 960 * 4) / denominator
      let end = 0
      for (const track of Object.values(view.project.tracks))
        for (const placement of Object.values(track?.placements ?? {}))
          if (placement && 'ticks' in placement.span) end = Math.max(end, placement.start + placement.span.ticks.length)
      return Math.max(1, Math.ceil(end / bar))
    },
    job: (request) => this.core?.exportJob(request) ?? null,
    projectName: () => this.project.view?.project.name ?? '',
    hasContent: () => Object.values(this.project.view?.project.tracks ?? {}).some((track) => Object.keys(track?.placements ?? {}).length > 0),
    hasLoop: () => (this.project.view?.project.transport.loop_region ?? null) !== null,
    folder: async () => (await this.storage?.folderHandle()) ?? null,
  }

  private refuseOpen(reason: OpenRefusal): OpenOutcome {
    this.state.notice = refusalNotice(reason)
    return { status: 'refused', reason }
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
      // A new engine (and the core's new slot table) holds no take: a take in progress is lost.
      this.recorder?.engineReplaced()
      if (report.status === 'encoded') {
        web.push(engine)
        // The snapshot gave the engine its slots again (a new epoch): the view carries them for the launch grid.
        this.refresh()
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
      onCommand: (command, timeStamp) => {
        if (this.state.mode === 'web') this.runPlayed(command, timeStamp)
      },
      onActivity: (note, velocity) => this.markNote(note, velocity),
    })
    this.state.midiState = (await this.midi.enable()) ? 'ok' : 'denied'
  }

  /**
   * A note played on a MIDI input or the computer keyboard at `timeStamp` (the event's time on the
   * page's performance clock): while the Web engine plays it is stamped with the audio context's
   * frame of that instant (`frame`), so a recording places it where it was played (#499). In Boost
   * the page's clock means nothing to the helper's engine: the note goes unstamped.
   */
  runPlayed(command: Command, timeStamp?: number): boolean {
    const frame = !this.native && this.web ? this.web.frameAt(timeStamp) : null
    return this.run(withFrame(command, frame))
  }

  /**
   * The single entry point for user actions. The project core applies the command; the active
   * engine gets what it means for it; the project store gets the events; the page-local
   * effects (`effects.ts`) follow. A command the project refuses changes nothing.
   */
  run(command: Command, options: { quiet?: boolean; author?: string } = {}): boolean {
    // A query that waits for the storage: it answers asynchronously, with the lists it read.
    if (isListStored(command)) {
      void this.listStored().catch((error: unknown) => console.error(error))
      return true
    }
    // Replaces the project: the storage asks before the core does, whoever sends it (a click or an agent).
    // The question is a dialog of the page and needs no click. Works before audio starts, like opening.
    if (isNewProject(command)) {
      // Without a storage (a bare dispatcher) nothing is kept anywhere, so there is nothing to ask about.
      // The name an agent gave, if any; else the page names the draft.
      const name = newProjectName(command)
      void (this.storage ? this.storage.newProject(name) : this.newProject(name)).catch((error: unknown) => console.error(error))
      return true
    }
    // A command the recorder answers otherwise (an undo during a take drops the take, #470).
    const instead = this.recorder?.instead(command)
    if (instead) return this.run(instead, options)
    const refusal = this.recorder?.refusal(command)
    if (refusal) {
      this.state.notice = refusal
      return false
    }
    const core = this.core
    // The banner that offers the project of last time shows on a fresh page, before any core exists: the
    // core is made, then the command runs (a click's permission to prompt outlasts the few milliseconds).
    if (!core && isStoredProjectCommand(command)) {
      void this.ensureCore().then(
        () => this.run(command, options),
        (error: unknown) => console.error(error),
      )
      return true
    }
    // No engine (audio not started, or stopped after a refused stream): nothing to keep in step.
    // Saving needs none: it is about the folder.
    if (!core || (!this.engine && !isSaveCommand(command))) return false
    const step = engineStep(() => core.run(command, options.author))
    if ('refused' in step) {
      // The views do not send what a gesture rules out by itself (an occupied cell sends nothing),
      // so a refusal is the core's: the user is told, the developer message stays in the console.
      console.warn('the project refused a command', command, step.refused)
      if (!options.quiet) this.showRefusal(step.refused)
      return false
    }
    this.mirror(step.events)
    this.recorder?.ran(command, 'output' in step ? step.output : undefined, step.events)
    if ('streamError' in step) {
      // No report (it was lost): the command may have applied, read the project again.
      if (step.events.length === 0) this.refresh()
      // Applied, but its engine messages did not encode: the Web engine is behind the project.
      // The helper takes the command itself, whatever its mirror of the stream.
      if (this.native) {
        if (!isStorageCommand(command)) this.native.send(command)
      } else {
        console.error(`engine stream: error ${step.streamError}`)
        void this.stopWeb(step.streamError, 0)
      }
    } else if (!(this.native && isStorageCommand(command))) this.engine?.send(command, step.push)
    applyEffects(command, {
      mode: this.state.mode,
      midi: this.midi,
      rememberWebInput: (id) => (this.lastWebInput = id),
      markNote: (note, velocity) => this.markNote(note, velocity),
      clearNotes: () => (this.state.activeNotes = []),
      save: () => void this.storage?.save(),
      saveAs: () => void this.storage?.saveAs(),
      exportPack: () => void this.storage?.exportPack(),
      importPack: () => void this.storage?.importPack(),
      openUrl: (url) => void this.storage?.openUrl(url),
      reopenLast: () => void this.storage?.reopenLast(),
      openFolder: () => void this.storage?.openFolder(),
      openRecent: (id) => void this.storage?.openRecent(id),
      openBrowserProject: (dir) => void this.storage?.openBrowserProject(dir),
      deleteBrowserProject: (dir) => void this.storage?.deleteBrowserProject(dir),
      keepMine: () => void this.storage?.keepMine(),
      loadFolderVersion: () => void this.storage?.loadDisk(),
      allowAccess: () => void this.storage?.resume(),
      exportAudio: (request) => this.audioExport?.request(request),
      cued: () => this.project.cued?.(),
    })
    // Whatever the command did, the folder may now lack changes: the storage writes them soon.
    this.storage?.changed()
    return true
  }

  /** The undo history for the history panel; empty before audio starts (no core yet). */
  history(): HistoryView {
    return this.core?.history() ?? { undo: [], redo: [], activity: [] }
  }

  /**
   * `project.list_stored`: the storage reads the folders and the browser's projects again, stages what it
   * found in the core (`Context::stored`) and the command answers with it. The projects the page can open,
   * newest first. The core is made when none exists yet (the page lists before audio starts).
   */
  async listStored(): Promise<StoredProjects> {
    const core = await this.ensureCore()
    if (this.storage) {
      await this.storage.refreshStored()
      core.stageStored({
        recent: this.storage.recent.map(({ id, name }) => ({ id, name })),
        browser: this.storage.browserProjects.map(({ dir, name }) => ({ dir, name })),
      })
    }
    const { report } = core.run(listStoredCommand())
    if (report.status !== 'applied' || !('recent' in report.output)) {
      throw new Error(`project.list_stored: ${JSON.stringify(report)}`)
    }
    return { recent: report.output.recent, browser: report.output.browser }
  }

  private refusalTimer: ReturnType<typeof setTimeout> | null = null

  /**
   * Tells the user a command was refused: one toast at a time. A refusal replaces the one shown;
   * the same one again only raises its count, so a burst never piles up. A report that is not a
   * refusal (a malformed command, a page bug) shows nothing.
   */
  private showRefusal(report: RunReport) {
    if (report.status !== 'refused') return
    const next = refusalOf(report.error)
    const shown = this.state.refusal
    const same = shown && shown.key === next.key && shown.whatKey === next.whatKey && shown.vars.max === next.vars.max
    this.state.refusal = { ...next, count: same ? shown.count + 1 : 1 }
    if (this.refusalTimer) clearTimeout(this.refusalTimer)
    this.refusalTimer = setTimeout(() => {
      this.refusalTimer = null
      this.state.refusal = null
    }, REFUSAL_TOAST_MS)
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
    const found: HelperState = result.status === 'available' ? 'available' : result.status === 'incompatible' ? 'incompatible' : 'absent'
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
      this.state.pairingMessage = result.attemptsLeft > 0 ? { key: 'helper.refused', n: result.attemptsLeft } : { key: 'helper.refusedNew' }
    } else {
      this.state.pairingMessage = { key: 'helper.failed' }
    }
  }

  /** The next probe of the wait for the helper's audio output (#525); `null` while the page does not wait. */
  private outputTimer: ReturnType<typeof setTimeout> | null = null
  /** When the probes of the wait began to see an output, in a row; `null` when the last one saw none. */
  private outputSeenSince: number | null = null
  /** Outputs lost soon after Boost began, in a row (`OUTPUT_FLAP_MS`): each doubles how long an output must stay before the page goes back. */
  private outputFlaps = 0
  /** When the current helper session became the engine; `null` in Web mode. */
  private boostSince: number | null = null
  /** Counts the ends of a wait asked by the user ("Stay in Web"), so a switch under way does not start the wait again. */
  private outputWaits = 0

  /**
   * The helper runs but has no audio output (#525): the page keeps playing itself and asks the
   * helper every `OUTPUT_POLL_MS` (its probe, which leaves the helper state as it is) until it has
   * one, then enters Boost. After flaps the output must be seen for a while first; a hidden page
   * waits until it is shown, so that a background tab never takes the helper from the one in use.
   * A helper that is gone or of another version ends the wait.
   */
  private awaitOutput() {
    if (this.native) return
    this.state.awaitingOutput = true
    this.outputSeenSince = null
    this.scheduleOutputProbe()
  }

  private scheduleOutputProbe() {
    if (this.outputTimer) clearTimeout(this.outputTimer)
    this.outputTimer = setTimeout(() => {
      this.outputTimer = null
      void this.probeOutput()
    }, OUTPUT_POLL_MS)
  }

  private async probeOutput() {
    if (!this.state.awaitingOutput || this.native) return this.stopAwaitingOutput()
    const result = await probeHelper()
    // "Stay in Web", or Boost entered by a click, meanwhile.
    if (!this.state.awaitingOutput) return
    if (this.native) return this.stopAwaitingOutput()
    if (result.status !== 'available') {
      // Another helper may come: its flaps start from none.
      this.stopAwaitingOutput()
      this.outputFlaps = 0
      void this.checkHelper()
      return
    }
    if (!result.audioOutput) {
      this.outputSeenSince = null
      return this.scheduleOutputProbe()
    }
    const now = Date.now()
    this.outputSeenSince ??= now
    const steady = this.outputFlaps === 0 ? 0 : Math.min(OUTPUT_STEADY_MAX_MS, OUTPUT_POLL_MS * 2 ** this.outputFlaps)
    const hidden = typeof document !== 'undefined' && document.hidden
    if (hidden || now - this.outputSeenSince < steady) return this.scheduleOutputProbe()
    // A refusal for want of an output starts the wait again (`connect`).
    this.stopAwaitingOutput()
    await this.enterBoost()
  }

  private stopAwaitingOutput() {
    if (this.outputTimer) clearTimeout(this.outputTimer)
    this.outputTimer = null
    this.outputSeenSince = null
    this.state.awaitingOutput = false
  }

  /** "Stay in Web": the page stops waiting for the helper's audio output and plays itself until Boost is asked again. */
  stayInWeb() {
    this.outputWaits++
    this.stopAwaitingOutput()
    this.outputFlaps = 0
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
      // A take in progress stays with the Web engine that captured it: it is dropped before the helper takes over (#470).
      this.recorder?.abandon()
      // The Web engine plays until now: it gets its notes released before the helper takes over.
      releaseAllNotes((command) => this.run(command))
      this.state.activeNotes = []
      this.native = session
      this.boosts++
      this.boostSince = Date.now()
      this.stopAwaitingOutput()
      this.state.mode = 'native'
      this.state.status = session.status
      this.state.helperUpdate = session.update
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
    } catch (error) {
      if (error instanceof NoOutputError) {
        // The helper runs and took the token, but has no audio output yet: the Web engine plays
        // on, the pairing stays, and the page waits for the output (#525).
        this.state.helper = 'available'
        this.awaitOutput()
        return
      }
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
    this.boostSince = null
    this.state.mode = 'web'
    this.state.helperUpdate = null
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
