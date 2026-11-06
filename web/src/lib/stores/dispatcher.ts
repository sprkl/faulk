import type { Engine, EngineEvents, HelperExportMessage } from '../engine/engine'
import { probeHelper, pairHelper } from '../engine/helper-client'
import { peakOf } from '../engine/feedback'
import { LoadMeter, sameLoad } from '../engine/load'
import { NativeEngine, NoOutputError, type HelperPlace } from '../engine/native-engine'
import { WebEngine } from '../engine/web-engine'
import { withFrame } from '../engine/clock'
import { decodeMidi } from '../midi/decode'
import type { AgentResult, AudioMedia, Command, HistoryView, ProjectEvent, RunReport } from '../generated/protocol'
import { WebMidiHost } from '../midi/web-midi'
import { draftName } from '../project/draft-name'
import { CoreError, coreModule, engineStep, ProjectCore, type EngineStep, type FolderEntry, type StoredProjects } from '../project/core'
import {
  analyzeRange,
  cancelImportOutput,
  isCancelImportPack,
  isListStored,
  isMixerLevels,
  isNewProject,
  isSaveCommand,
  isStoredProjectCommand,
  listStored as listStoredCommand,
  newProject as newProjectCommand,
  newProjectName,
  renameProject as renameProjectCommand,
} from '../project-commands'
import type { AudioExport, ExportHost } from '../export/audio-export.svelte'
import { browserCanAsk, importCommand, type AudioImport, type ImportPlacer } from '../audio/import/audio-import.svelte'
import { audioNameOf } from '../audio/import/from-url'
import { sortedTracks } from '../project/tracks'
import { importAudioUrlOutput, isImportAudioUrl, type ImportAudioUrl } from '../clip-commands'
import { audioMediaOf } from '../audio/import/media'
import type { HelperLinkWrite, ProjectHost, ProjectStorage } from '../storage/storage.svelte'
import { releaseAllNotes, requestMidiInputs } from './housekeeping'
import { takePairLink } from '../pair-link'
import { agentAuthor, answerAgentCall } from '../agents/agent-call'
import { LevelHold, type RenderAnalyzer } from '../agents/levels'
import { AgentConfirmations } from '../agents/confirm.svelte'
import { agentQuestion, confirmDeadline, isStorageGuarded, notConfirmed, type GuardView } from '../agents/guard'
import { ControlIncompatible, ControlLink, ControlTimeout } from '../agents/control-link'
import { applyEffects } from './effects'
import type { EngineStore, HelperState } from './engine.svelte'
import { refusalNotice, type OpenOutcome, type OpenRefusal } from './open'
import type { ProjectStore } from './project.svelte'
import type { PresetLibrary } from '../storage/presets'
import type { PresetList } from '../presets'
import { listPresets as listPresetsCommand } from '../preset-commands'
import type { Recorder } from './recorder.svelte'
import { refusalOf } from './refusal'
import { storage, TOKEN_KEY } from './storage'
import { isSetTheme, isThemeChoice, SET_THEME_OUTPUT, type ThemeChoice } from '../theme'

/** How long a link's import answers a retry of the same call with its clip (#664), in ms: 10 minutes. */
export const LINK_IMPORT_KEPT_MS = 10 * 60_000

/** How `run` runs a command: `quiet` shows no refusal, `author` is the history's (`user` by default). */
type RunOptions = {
  quiet?: boolean
  author?: string
  playedByHelper?: boolean
  /** The user already allowed the agent that sends it (AG-4): the storage asks nothing more. */
  allowed?: boolean
  /** Told the storage action the command started, for an agent's answer to wait for it. */
  storageRun?: StorageRun
}

/** The storage action a command started (`project.open_recent`, `project.delete_browser_project`…), `null` for none. */
type StorageRun = { done: Promise<boolean> | null }

/** A link marker for the helper (AU-7): 128 random bits as 32 lowercase hex digits (`helper::disk::valid_nonce`). */
export function randomNonce(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('')
}

/** What the agents' questions need from the storage (`GuardView`). */
function guardView(storage: ProjectStorage): GuardView {
  return {
    recent: storage.recent,
    browserProjects: storage.browserProjects,
    reopenTarget: storage.reopen ?? storage.failedReopen,
    folderName: storage.folder?.name ?? null,
    inConflict: storage.inConflict,
  }
}

/** What running a command did: an agent's answer, or `started` for a command the page answers later. */
type Ran = AgentResult | { status: 'started' }
const STARTED: Ran = { status: 'started' }

/** The epoch of the first engine stream of the page's project core (newer than a fresh engine's 0). */
const FIRST_EPOCH = 1

/** While the page waits for the helper's audio output (#525): how often it asks the helper, in milliseconds. */
export const OUTPUT_POLL_MS = 2000
/** A Boost session that loses its output sooner than this after it began counts as a flap: the next return waits longer. */
export const OUTPUT_FLAP_MS = 60_000
/** The longest the output must stay before the page goes back to Boost after flaps. */
export const OUTPUT_STEADY_MAX_MS = 60_000

/** The helper's engine is resynced at most this many times within `NATIVE_RESYNC_WINDOW_MS`; past that the page plays itself (BO-1). */
export const NATIVE_RESYNCS = 3
export const NATIVE_RESYNC_WINDOW_MS = 10_000

/**
 * After the control link is lost or could not open (#608), the page probes the helper again after each of these delays
 * in turn, then every last one until a link is open again, in milliseconds: a helper that restarts (an update, a sleep
 * of the machine) serves the agents again without a reload.
 */
export const CONTROL_RETRY_MS = [1000, 2000, 4000, 8000, 16_000, 30_000]

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
  /** Who listens to the helper's export messages (the export dialog, #319): told `null` when the session ends. */
  private exportListener: ((message: HelperExportMessage | null) => void) | null = null
  private midi: WebMidiHost | null = null

  constructor(
    private readonly project: ProjectStore,
    private readonly state: EngineStore,
  ) {}

  /** MIDI recording (#470): told of every command applied and every report of the current engine. */
  recorder: Recorder | null = null

  /** Dev checks only (#733): the Web engine's clock, `null` when it does not play. */
  webClock(): { sampleRate: number; heardFrameAt(timeStamp: number): number | null } | null {
    const web = this.web
    return !this.native && web ? { sampleRate: web.sampleRate, heardFrameAt: (t) => web.heardFrameAt(t) } : null
  }

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
    // The frames the output protection silenced (#667) count from 0 for the page's own engine, and from the first
    // report of a helper session: the helper's engine outlives a page, an earlier page's silence is not this one's.
    let silencedBase: number | null = kind === 'web' ? 0 : null
    return {
      onStatus: (status) => {
        if (current()) this.state.status = status
      },
      onFeedback: (report) => {
        if (!current()) return
        this.state.feedback = report
        silencedBase ??= report.silencedFrames
        // A count below the base: the helper built a new engine (another output), whose count starts at 0.
        if (report.silencedFrames < silencedBase) silencedBase = 0
        const silenced = report.silencedFrames - silencedBase
        if (this.state.silencedFrames !== silenced) this.state.silencedFrames = silenced
        const shown = load.update(report.cpuLoad, report.cpuPeak, performance.now())
        if (!sameLoad(this.state.load, shown)) this.state.load = shown
        this.state.peak = peakOf(report.master)
        this.levelHold.feed(report, performance.now())
        this.state.voices = report.voices
        this.recorder?.feedback(report)
      },
      onMidiInputs: (inputs, selected) => {
        if (kind !== 'native' || !current()) return
        this.state.inputs = inputs
        this.state.selectedInput = selected
      },
      onMidiMessage: (data, played) => {
        // The helper's input runs in the page's project, as a Web MIDI message (BO-1); a note the helper
        // already played with the page's table (#598) sends no engine bytes, so it never sounds twice.
        if (kind !== 'native' || !current()) return
        const command = decodeMidi(data)
        if (command) this.runPlayed(command, undefined, played)
      },
      onHelperUpdate: (update) => {
        if (kind === 'native' && current()) this.state.helperUpdate = update
      },
      onAgentCall: (agent, command, answer, signal) => {
        // An agent's call runs only on the session the page plays: one that is not (yet) current, or no
        // longer, answers at once rather than leaving the agent to wait (AG-1).
        if (kind !== 'native' || !current()) {
          answer({ status: 'unavailable', reason: 'the page is switching between its own engine and the helper: retry' })
          return
        }
        void answerAgentCall(this, agent, command, signal).then(answer)
      },
      // Whichever session tells it, even one no longer played: the agent left all the same (#641).
      onAgentLeft: (agent) => this.agentLeft(agent),
      onFolderState: (project, state) => {
        // A state about another project than the one open now (the page moved on since) is stale (AU-7).
        if (kind !== 'native' || !current()) return
        if (project !== null && project !== (this.project.view?.project.id ?? null)) return
        this.state.helperFolder = { project, state }
      },
      onExport: (message) => {
        // The helper's export messages go to the export of the session the page plays (#319).
        if (kind !== 'native' || !current()) return
        this.exportListener?.(message)
      },
      onStreamRefused: (code, at, epoch) => {
        if (!current()) return
        if (kind === 'native') this.resyncNative(code, epoch)
        else void this.stopWeb(code, at)
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
        this.helperExportLost()
        this.linkLost()
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
      web.setFolder?.(this.folder)
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
    if (this.core) return this.core
    this.core = ProjectCore.create(module, 'single_synth', FIRST_EPOCH, undefined, this.nameNew())
    // The user's preset library is every project's: the core gets it as soon as it exists (#191).
    void this.loadLibrary().catch((error: unknown) => console.error(error))
    return this.core
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
      // The helper plays the opened project (BO-1): the snapshot of the new project, in the core's next
      // epoch (newer than the helper's), starts its engine over. The Web engine catches up when it plays again.
      if ('streamError' in step) this.resyncNative(step.streamError)
      else {
        this.native.push(step.push)
        this.routeNative(this.native)
      }
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
    renameProject: (name) => this.run(renameProjectCommand(name), { quiet: true }),
    packExport: async (entries, io) => (await this.ensureCore()).packExport(entries, io),
    packImport: async (zip) => (await this.ensureCore()).packImport(zip),
    prepare: async () => void (await this.ensureCore()),
    audioMedia: () => audioMediaOf(this.project.view?.project),
    folderChanged: (handle) => {
      this.folder = handle
      this.web?.setFolder?.(handle)
      // In Boost, the helper reads the audio files of the project's new place (AU-7).
      void this.tellHelperPlace()
    },
  }

  /** The project's folder, given to each Web engine for its disk side (AU-3). */
  private folder: FileSystemDirectoryHandle | null = null

  /** The link marker this page wrote for the helper (AU-7), and the folder it is in: one per folder attached. */
  private helperLink: { folder: FileSystemDirectoryHandle; nonce: string; name: string } | null = null
  /** The marker write under way, and its folder: a second caller for that folder waits for it instead of writing another. */
  private linkWrite: { folder: FileSystemDirectoryHandle; done: Promise<HelperLinkWrite> } | null = null

  /**
   * The project's place as the helper is told it (AU-7, #116): its id and the link marker the page wrote
   * in its real folder (written once per folder attached, one write at a time), so that the helper, which
   * never gets a path, opens that folder and not a copy. `no_folder` for a project without a real folder
   * (kept in the browser); `failed` when the folder refused the marker (its permission not granted yet).
   */
  private async helperPlace(): Promise<HelperPlace | 'no_folder' | 'failed'> {
    const folder = this.folder
    const project = this.project.view?.project.id
    if (!folder || !project || !this.storage) return 'no_folder'
    if (this.helperLink?.folder !== folder) {
      if (this.linkWrite?.folder !== folder) {
        const nonce = randomNonce()
        const storage = this.storage
        const done = storage.writeHelperLink(nonce).then((written) => {
          if (written.status === 'written' && this.folder === folder) this.helperLink = { folder, nonce, name: written.name }
          return written
        })
        const write = { folder, done }
        this.linkWrite = write
        void done.finally(() => {
          if (this.linkWrite === write) this.linkWrite = null
        })
      }
      const written = await this.linkWrite!.done
      // Another place since, or a browser project: nothing to tell for this folder.
      if (this.folder !== folder || written.status === 'no_folder') return 'no_folder'
      if (written.status === 'failed' || this.helperLink?.folder !== folder) return 'failed'
    }
    return { project, nonce: this.helperLink!.nonce, name: this.helperLink!.name }
  }

  /** The Boost session ended: the helper's export it carried ends with it (#319). */
  private helperExportLost() {
    const listener = this.exportListener
    this.exportListener = null
    listener?.(null)
  }

  /**
   * Tells the helper session where the project lives now (`folder.open` or `folder.close`, AU-7). A marker
   * the folder refused closes the helper's folder too (it never keeps reading a folder the page left), is
   * said in the Clips panel, and is tried again when the user allows the access.
   */
  private async tellHelperPlace() {
    const native = this.native
    if (!native) return
    const place = await this.helperPlace()
    if (this.native !== native) return
    this.state.helperLinkFailed = place === 'failed'
    native.place(typeof place === 'string' ? null : place)
  }

  /**
   * `project.choose_helper_folder` (AU-7): the helper shows its folder dialog for the project's folder. It
   * needs Boost, the user's click (an agent's call without one gets a notice) and a real folder whose
   * marker the page could write.
   */
  private async chooseHelperFolder() {
    const native = this.native
    if (!native) {
      this.state.notice = 'helper.folderNotBoost'
      return
    }
    if (!browserCanAsk()) {
      this.state.notice = 'helper.folderNeedsClick'
      return
    }
    const place = await this.helperPlace()
    if (this.native !== native) return
    this.state.helperLinkFailed = place === 'failed'
    if (typeof place === 'string') native.place(null)
    if (place === 'no_folder') this.state.notice = 'helper.folderInBrowser'
    else if (place === 'failed') this.state.notice = 'helper.folderNoMarker'
    else {
      // The place the dialog is for is the one the helper knows: told first when it was not (a retry after a refused marker).
      native.place(place)
      native.chooseFolder(place)
    }
  }

  /** Stages an imported audio file in the core (AU-1), for `clip.import_audio`; false while there is no core. */
  stageMedia(media: AudioMedia): boolean {
    if (!this.core) return false
    this.core.stageMedia(media)
    return true
  }

  /** Importing audio files (AU-1); set once by `createStores`. */
  audioImport: AudioImport | null = null

  /** The project's storage; set once by `createStores`. */
  storage: ProjectStorage | null = null

  /** The audio export (#297); set once by `createStores`. */
  audioExport: AudioExport | null = null

  /** What the engine's reports showed since the last `mixer.levels` (#665). */
  readonly levelHold = new LevelHold()

  /** The offline measure of `project.analyze_render` (#665); set once by `createStores`. */
  analyzer: RenderAnalyzer | null = null

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
    mediaFolder: () => this.folder,
    analysisRunning: () => this.analyzer?.busy ?? false,
    helperExport: () => {
      // Boost, a helper that exports, and the project's folder open in it (AU-7): the helper writes the file (#319).
      const native = this.native
      const folder = this.state.helperFolder
      const project = this.project.view?.project.id ?? null
      if (!native || !native.capabilities?.export || !folder || folder.state.status !== 'open' || folder.project !== project) return null
      return {
        start: (start) => native.exportStart(start),
        cancel: (id) => native.exportCancel(id),
        listen: (listener) => {
          this.exportListener = listener
          return () => {
            if (this.exportListener === listener) this.exportListener = null
          }
        },
      }
    },
    projectId: () => this.project.view?.project.id ?? '',
    mediaNames: (hashes) => {
      const names = new Map<string, string>()
      for (const track of Object.values(this.project.view?.project.tracks ?? {}))
        for (const clip of Object.values(track?.clips ?? {}))
          if (clip && 'audio' in clip.content) names.set(clip.content.audio.media.hash, clip.content.audio.media.name)
      return hashes.map((hash) => names.get(hash) ?? hash)
    },
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
    // The engine that captured a take is gone, and no report will settle a take waiting to be written (#513).
    this.recorder?.engineReplaced()
    this.state.streamError = { code, at }
    this.state.started = false
    this.state.status = null
    this.state.peak = 0
    this.state.voices = 0
    this.state.silencedFrames = 0
    this.state.activeNotes = []
    await web.close().catch((error: unknown) => console.error(error))
  }

  /**
   * Starts the helper's engine over from the project (BO-1): a `Reset` and the snapshot, in an epoch newer
   * than `after` (the helper's engine outlives a page, so its epoch may be ahead of the core's). False when
   * the snapshot did not encode.
   */
  private syncNative(native: NativeEngine, after: number): boolean {
    const core = this.core
    if (!core) return false
    try {
      const { report, engine } = core.snapshotAfter(after)
      // A new slot table holds no take: a take in progress is lost (as for the Web engine).
      this.recorder?.engineReplaced()
      if (report.status !== 'encoded') return false
      native.push(engine)
      // The table of the helper's MIDI input in the new epoch (#598), right behind the snapshot.
      this.routeNative(native)
      // The snapshot gave the engine its slots again (a new epoch): the view carries them for the launch grid.
      this.refresh()
      return true
    } catch (error) {
      console.error(error)
      return false
    }
  }

  /** When the helper last had to be resynced, for `resyncNative`'s bound. */
  private nativeResyncs: number[] = []

  /**
   * The helper's engine is behind the project (it refused a stream, or a command's messages did not
   * encode): it starts over from a snapshot newer than `epoch` (the one the helper reported). A helper
   * that keeps refusing (`NATIVE_RESYNCS` within `NATIVE_RESYNC_WINDOW_MS`), or a snapshot that does not
   * encode, sends the page back to Web, which plays the project itself, with the audio error notice.
   */
  private resyncNative(code: number, epoch?: number) {
    const native = this.native
    if (!native) return
    console.error(`helper engine stream: error ${code}`)
    const now = Date.now()
    this.nativeResyncs = this.nativeResyncs.filter((at) => now - at < NATIVE_RESYNC_WINDOW_MS)
    if (this.nativeResyncs.length < NATIVE_RESYNCS) {
      this.nativeResyncs.push(now)
      if (this.syncNative(native, epoch ?? native.epoch)) return
    }
    this.state.notice = 'audio.error'
    void this.enterWeb()
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
   * `playedByHelper`: a note of the helper's input that the helper already played (#598): it runs in
   * the project (keys shown, recording, held notes) but its engine bytes are not sent.
   */
  runPlayed(command: Command, timeStamp?: number, playedByHelper = false): boolean {
    const frame = !this.native && this.web ? this.web.frameAt(timeStamp) : null
    return this.run(withFrame(command, frame), { playedByHelper })
  }

  /**
   * Tells the helper where its MIDI input plays now (#598): the core's table (the selected and armed MIDI
   * tracks, by slot, in the core's epoch). `NativeEngine.route` sends it only when it changed.
   */
  private routeNative(native: NativeEngine) {
    const core = this.core
    if (!core) return
    try {
      native.route(core.liveRouting())
    } catch (error) {
      // No table: the helper keeps the one it has, or plays nothing and the page plays the notes.
      console.error(error)
    }
  }

  /**
   * The single entry point for user actions. The project core applies the command; the active
   * engine gets what it means for it; the project store gets the events; the page-local
   * effects (`effects.ts`) follow. A command the project refuses changes nothing.
   */
  run(command: Command, options: RunOptions = {}): boolean {
    const ran = this.execute(command, options)
    return ran.status === 'applied' || ran.status === 'started'
  }

  /**
   * Runs an agent's command (AG-1) through the same path as a user's (`run`): the history records it as
   * `author`, its refusals go back to the agent (no toast), its undo is the user's. The commands the page
   * answers asynchronously are awaited: `project.list_stored` answers its lists, `project.new` its name once
   * the storage made it, and the storage actions an agent must have allowed (`STORAGE_GUARDED`) once the storage
   * did them, or a refusal when it did not.
   *
   * A destructive command, or a storage action that replaces or deletes a project or settles a conflict, first waits
   * for the user's answer in the page (AG-4, `agentQuestion`): until `AGENT_CONFIRM_MS` after the call reached the
   * page. Anything but "allow" answers a typed refusal (`confirmation.declined`, `confirmation.timeout`) and nothing
   * runs; `signal` (the call's link closed) withdraws the question. Once allowed, the storage asks nothing more.
   */
  async runForAgent(command: Command, author: string, signal?: AbortSignal): Promise<AgentResult> {
    const deadline = confirmDeadline(this.confirmations.clock.now())
    if (isListStored(command)) {
      const lists = await this.listStored()
      return { status: 'applied', output: lists }
    }
    // Needs no project: the theme is the page's (#687).
    if (isSetTheme(command)) return this.setTheme(command.theme)
    // Needs no project either: the imports are the storage's, which says what it stopped (#735).
    if (isCancelImportPack(command)) return this.cancelImportPack()
    // An audio file from a link (#664): its own path, where the user's answer comes before any byte is fetched.
    if (isImportAudioUrl(command)) return this.importAudioUrl(command, author, deadline, signal)
    // The queries that measure levels: the page stages its numbers in the core, then the command answers (#665).
    if (isMixerLevels(command) && this.core) this.stageLevels(this.core)
    const range = analyzeRange(command)
    if (range) {
      const early = await this.measureRender(range, signal)
      if (early) return early
    }
    const question = agentQuestion(command, this.storage ? guardView(this.storage) : null)
    // Without a core, `execute` would make one and run the command later: nothing an agent can be told. Not asked
    // for either. `project.new` makes its own project.
    const noCore = !this.core && !isNewProject(command)
    if (noCore) return { status: 'unavailable', reason: 'the page has no project yet: start audio in the page first' }
    // A guarded action that asks nothing (a way out of a conflict while there is none) is not handed to the storage:
    // a conflict found meanwhile would make it ask the user with no deadline, for an action nobody allowed.
    if (!question && isStorageGuarded(command)) {
      return { status: 'refused', error: { code: 'not_done', message: 'the page did not do it: nothing changed', notice: 'storage.noConflict' } }
    }
    if (question) {
      const answer = await this.confirmations.ask(author, question, deadline, signal)
      if (answer !== 'confirmed') return notConfirmed(answer)
    }
    const allowed = question !== null
    if (isNewProject(command)) {
      const name = newProjectName(command)
      const made = await (this.storage ? this.storage.newProject(name, allowed) : this.newProject(name))
      if (!made) return { status: 'refused', error: { code: 'not_done', message: 'the page did not start a new project: nothing changed' } }
      const view = this.core?.project()
      return { status: 'applied', output: { name: view?.project.name ?? name ?? '' } }
    }
    // The storage action an allowed call starts: awaited, so that the agent learns whether it was done.
    const storageRun: StorageRun = { done: null }
    const notices: string[] = []
    const stopWatching = this.storage?.watchNotices((key) => notices.push(key))
    try {
      const ran = this.execute(command, { quiet: true, author, allowed, storageRun })
      // Only the commands answered above start without answering; none reaches here with a core.
      if (ran.status === 'started') return { status: 'unavailable', reason: 'the page answers this command later' }
      if (ran.status !== 'applied' || !storageRun.done) return ran
      let done: boolean
      try {
        done = await storageRun.done
      } catch (error) {
        console.error(error)
        return { status: 'unavailable', reason: `the page failed to do it: ${String(error)}` }
      }
      if (done) return ran
      // What the storage said meanwhile (a project gone, locked, no conflict…): a hint for the agent, the page's own
      // notice key; nothing changed either way.
      return {
        status: 'refused',
        error: { code: 'not_done', message: 'the page did not do it: nothing changed', notice: notices.at(-1) ?? null },
      }
    } finally {
      stopWatching?.()
    }
  }

  /**
   * `clip.import_audio_url` for an agent (#664). The core checks the track and the link's shape first, so that a wrong
   * call is refused before the user is asked; then the page checks the link with the browser's parser and its own host
   * (`url.refused`); then the user is asked, with the link's host and the file's name, until the call's `deadline`.
   * Nothing is sent to the host before "Allow". Only then is the file downloaded and imported as a picked file, its clip
   * made by `clip.import_audio` as `author`. The download and the import exist only here, after the answer: a `run` of
   * the command (`applyEffects` has no case for it) only gets the core's check. Every way it ends without a clip is a
   * typed refusal, nothing changed; `signal` (the call's link closed) stops it.
   */
  private async importAudioUrl(command: ImportAudioUrl, author: string, deadline: number, signal?: AbortSignal): Promise<AgentResult> {
    const audioImport = this.audioImport
    if (!this.core || !audioImport) return { status: 'unavailable', reason: 'the page has no project yet: start audio in the page first' }
    const checked = this.execute(command, { quiet: true, author })
    if (checked.status === 'started') return { status: 'unavailable', reason: 'the page answers this command later' }
    if (checked.status !== 'applied') return checked
    const link = audioImport.checkLink(command.url)
    if (link.status !== 'ok') {
      return {
        status: 'refused',
        error: {
          code: 'url.refused',
          message: 'the page does not fetch this link (https only; plain http only to this computer, from a page served by it): nothing changed',
        },
      }
    }
    // A second call for a link under way (an agent retrying after `page.timeout`) would import the file twice.
    if (audioImport.importing(link.url)) {
      return { status: 'refused', error: { code: 'import.busy', message: 'this link is already being imported: wait for its answer, then check track.get' } }
    }
    // The same call again, soon after it imported (its answer was lost to `page.timeout`): the clip it made, while it is
    // there; nothing is asked, downloaded or changed.
    const replay = this.linkReplay(link.url.href, command.track)
    if (replay) return replay
    const project = this.project.view?.project
    const entry = project ? sortedTracks(project).find((e) => e.id === command.track) : undefined
    const track = { name: entry?.track.name ?? '', index: entry?.index ?? 0 }
    const name = audioNameOf(link.url)
    const answer = await this.confirmations.ask(author, { kind: 'importAudio', host: link.host, name, track }, deadline, signal)
    if (answer !== 'confirmed') return notConfirmed(answer)
    // Two calls for the link asked at once: the first one allowed may have imported it meanwhile.
    const late = this.linkReplay(link.url.href, command.track)
    if (late) return late
    let clip: string | null = null
    const place: ImportPlacer = (media) => {
      const made = this.execute(importCommand(command.track, media.hash), { quiet: true, author })
      if (made.status !== 'applied') return false
      clip = (made.output as { clip: string }).clip
      return true
    }
    // The project the user allowed it for: another one opened meanwhile gets nothing of it.
    const allowedFor = project?.id
    const stillWanted = () => this.project.view?.project.id === allowedFor
    const outcome = await audioImport.importUrl(command.track, link.url, link.host, name, place, signal, stillWanted)
    if (signal?.aborted && outcome.status !== 'ok') return { status: 'unavailable', reason: 'the call ended before the file was imported: nothing changed' }
    if (outcome.status !== 'ok') return { status: 'refused', error: { code: outcome.code, message: outcome.message } }
    if (clip !== null && allowedFor) {
      const now = this.confirmations.clock.now()
      // Only the recent ones are kept: the map never grows past the links of the last 10 minutes.
      for (const [href, kept] of this.linkImports) if (now - kept.at >= LINK_IMPORT_KEPT_MS) this.linkImports.delete(href)
      this.linkImports.set(link.url.href, { project: allowedFor, track: command.track, clip, media: outcome.hash, at: now })
    }
    return { status: 'applied', output: importAudioUrlOutput(clip, outcome.hash) }
  }

  /** The answer of a link's import already done (#664): the same link and track, the same project, recent, its clip still there. */
  private linkReplay(href: string, track: string): AgentResult | null {
    const project = this.project.view?.project
    const done = this.linkImports.get(href)
    if (!done || !project || done.project !== project.id || done.track !== track) return null
    if (this.confirmations.clock.now() - done.at >= LINK_IMPORT_KEPT_MS || !project.tracks[track]?.clips[done.clip]) return null
    return { status: 'applied', output: importAudioUrlOutput(done.clip, done.media) }
  }

  /** Stages what the meters held since the last `mixer.levels` for the core to answer with. */
  private stageLevels(core: ProjectCore) {
    const slots = this.project.view?.slots
    const held = this.levelHold.take(performance.now(), this.state.started ? slots?.epoch : undefined)
    // What the output protection silenced (#667) is the engine's count, playing or not.
    core.stageLevels({ ...held, silenced_frames: this.state.silencedFrames })
  }

  /**
   * `project.analyze_render`: renders the range offline and stages what it measured in the core, so that the command
   * answers with it. `null` when the command is to run now; else the answer (refused range, busy, stopped, failed).
   * Nothing of the project changes, and a stop (the call's signal, the time cap) leaves nothing staged.
   */
  private async measureRender(range: { from_bar?: number; to_bar?: number }, signal?: AbortSignal): Promise<AgentResult | null> {
    const core = this.core
    const analyzer = this.analyzer
    if (!core || !this.engine || !analyzer) return { status: 'unavailable', reason: 'the page has no project yet: start audio in the page first' }
    if (analyzer.busy) return { status: 'unavailable', reason: 'another analysis is running: retry when it is done' }
    if (this.audioExport?.busy) return { status: 'unavailable', reason: 'the page is exporting a file: retry when it is done' }
    const job = core.exportJob({ analysis: true, ...range })
    if (job.status === 'refused') return { status: 'refused', error: job.error }
    if (job.status !== 'ready') return { status: 'unavailable', reason: `the project could not be prepared for a render (${job.stream_error})` }
    const outcome = await analyzer.run(job, this.folder, signal)
    switch (outcome.status) {
      case 'busy':
        return { status: 'unavailable', reason: 'another analysis is running: retry when it is done' }
      case 'aborted':
        return { status: 'unavailable', reason: 'the call was cancelled: nothing was measured' }
      case 'failed':
        return { status: 'unavailable', reason: outcome.reason }
      case 'done':
        // The project may have been replaced while it rendered: the command refuses a range that is no longer this one.
        core.stageAnalysis({
          start_tick: job.plan.start_tick,
          end_tick: job.plan.end_tick,
          slots: job.tracks,
          tracks: outcome.levels.tracks,
          master: outcome.levels.master,
          silenced_frames: outcome.levels.silenced,
          reduction: outcome.levels.reduction,
        })
        return null
    }
  }

  /** The page's questions before an agent's destructive call runs (AG-4), shown by `views/confirm/`. */
  readonly confirmations = new AgentConfirmations()

  /** The files imported from links lately (#664), by link: a retry of the same call answers the clip already made. */
  private linkImports = new Map<string, { project: string; track: string; clip: string; media: string; at: number }>()

  /** Where `view.set_theme` goes: the page's theme (`createStores` sets it); `null` in a bare dispatcher. */
  theme: { set(choice: ThemeChoice): void } | null = null

  /**
   * `view.set_theme` (#687): the core declares it and changes nothing, so the page answers it without one. An
   * agent's call is not checked by the core's schema on this path, hence the check of the value here.
   */
  private setTheme(choice: unknown): AgentResult {
    if (!isThemeChoice(choice)) return { status: 'invalid', message: 'the theme is "system", "dark" or "light"' }
    if (!this.theme) return { status: 'unavailable', reason: 'the page has no theme to set' }
    this.theme.set(choice)
    return { status: 'applied', output: SET_THEME_OUTPUT }
  }

  /**
   * `project.cancel_import_pack` for an agent: the imports the storage stopped, and those it could not (their project had
   * started to open). When none could be stopped and some were too late, the agent is told so instead of being answered as
   * if it had worked (#735).
   */
  private cancelImportPack(): AgentResult {
    const { cancelled, tooLate } = this.storage?.cancelImport() ?? { cancelled: 0, tooLate: 0 }
    if (cancelled === 0 && tooLate > 0) {
      return {
        status: 'refused',
        error: { code: 'not_done', message: 'too late: the pack is already opening its project, nothing was cancelled', notice: null },
      }
    }
    return { status: 'applied', output: cancelImportOutput(cancelled, tooLate) }
  }

  /** `run`, saying what happened: what an agent is answered (`runForAgent`). */
  private execute(command: Command, options: RunOptions): Ran {
    // The theme is the user's, kept by the browser (#687): the page applies it, before audio starts too.
    if (isSetTheme(command)) return this.setTheme(command.theme)
    // A query that waits for the storage: it answers asynchronously, with the lists it read.
    if (isListStored(command)) {
      void this.listStored().catch((error: unknown) => console.error(error))
      return STARTED
    }
    // Replaces the project: the storage asks before the core does, whoever sends it (a click or an agent).
    // The question is a dialog of the page and needs no click. Works before audio starts, like opening.
    if (isNewProject(command)) {
      // Without a storage (a bare dispatcher) nothing is kept anywhere, so there is nothing to ask about.
      // The name an agent gave, if any; else the page names the draft.
      const name = newProjectName(command)
      void (this.storage ? this.storage.newProject(name) : this.newProject(name)).catch((error: unknown) => console.error(error))
      return STARTED
    }
    // A command the recorder answers otherwise (an undo during a take drops the take, #470).
    const instead = this.recorder?.instead(command)
    if (instead) return this.execute(instead, options)
    const refusal = this.recorder?.refusal(command)
    if (refusal) {
      // Shown whoever asks, quiet runs and agents included (#513): the user sees why a take did not start;
      // an agent gets the same reason as its answer.
      this.state.notice = refusal
      return { status: 'refused', error: { code: refusal } }
    }
    const core = this.core
    // The banner that offers the project of last time shows on a fresh page, before any core exists: the
    // core is made, then the command runs (a click's permission to prompt outlasts the few milliseconds).
    if (!core && isStoredProjectCommand(command)) {
      void this.ensureCore().then(
        () => this.run(command, options),
        (error: unknown) => console.error(error),
      )
      return STARTED
    }
    // No engine (audio not started, or stopped after a refused stream): nothing to keep in step.
    // Saving needs none: it is about the folder.
    if (!core || (!this.engine && !isSaveCommand(command))) {
      return {
        status: 'unavailable',
        reason: core ? 'the page plays no engine: start audio in the page' : 'the page has no project yet: start audio in the page',
      }
    }
    const step = engineStep(() => core.run(command, options.author))
    if ('refused' in step) {
      // The views do not send what a gesture rules out by itself (an occupied cell sends nothing),
      // so a refusal is the core's: the user is told, the developer message stays in the console.
      console.warn('the project refused a command', command, step.refused)
      if (!options.quiet) this.showRefusal(step.refused)
      const report = step.refused
      return report.status === 'refused'
        ? { status: 'refused', error: report.error }
        : { status: 'invalid', message: report.status === 'invalid' ? report.message : 'not a command' }
    }
    this.mirror(step.events)
    this.recorder?.ran(command, 'output' in step ? step.output : undefined, step.events)
    this.writePresets(step.events)
    if ('streamError' in step) {
      // No report (it was lost): the command may have applied, read the project again.
      if (step.events.length === 0) this.refresh()
      // Applied, but its engine messages did not encode: the engine is behind the project. The
      // helper's catches up by snapshot; the Web engine stops (the restart starts a new one).
      if (this.native) this.resyncNative(step.streamError)
      else {
        console.error(`engine stream: error ${step.streamError}`)
        void this.stopWeb(step.streamError, 0)
      }
    } else if (!options.playedByHelper) this.engine?.send(command, step.push)
    // Any command may change where the live input goes (selection, arming, a track created or deleted, an
    // undo): the helper gets the table again when it changed (#598).
    if (this.native) this.routeNative(this.native)
    // A storage action an agent's answer waits for (AG-4); a user's runs on its own, as before.
    const allowed = options.allowed ?? false
    const track = (run: Promise<boolean> | undefined) => {
      if (run && options.storageRun) options.storageRun.done = run
      else void run
    }
    applyEffects(command, {
      mode: this.state.mode,
      midi: this.midi,
      rememberWebInput: (id) => (this.lastWebInput = id),
      markNote: (note, velocity) => this.markNote(note, velocity),
      clearNotes: () => (this.state.activeNotes = []),
      save: () => void this.storage?.save(),
      saveAs: () => void this.storage?.saveAs(),
      exportPack: () => void this.storage?.exportPack(),
      cancelExportPack: () => this.storage?.cancelExport(),
      cancelImportPack: () => this.storage?.cancelImport(),
      importPack: () => void this.storage?.importPack(),
      openUrl: (url) => track(this.storage?.openUrl(url, allowed)),
      reopenLast: () => track(this.storage?.reopenLast(allowed)),
      openFolder: () => void this.storage?.openFolder(),
      openRecent: (id) => track(this.storage?.openRecent(id, allowed)),
      openBrowserProject: (dir) => track(this.storage?.openBrowserProject(dir, allowed)),
      deleteBrowserProject: (dir) => track(this.storage?.deleteBrowserProject(dir, allowed)),
      keepMine: () => track(this.storage?.keepMine(allowed)),
      loadFolderVersion: () => track(this.storage?.loadDisk(allowed)),
      // Once the folder can be written again, the helper's marker too (AU-7).
      allowAccess: () => void this.storage?.resume().then(() => this.tellHelperPlace()),
      chooseHelperFolder: () => void this.chooseHelperFolder(),
      exportAudio: (request) => this.audioExport?.request(request),
      chooseAudio: (track) => this.audioImport?.choose(track),
      cancelAudioImport: () => this.audioImport?.cancel(),
      cued: () => this.project.cued?.(),
    })
    // Whatever the command did, the folder may now lack changes: the storage writes them soon.
    this.storage?.changed()
    // A command whose engine bytes did not encode applied all the same; its report may be lost.
    return { status: 'applied', output: 'output' in step ? step.output : null }
  }

  // ---------------------------------------------------------------- presets (#191)

  /** The user's preset library (the browser's storage); set once by `createStores`, `null` in a bare dispatcher. */
  presetLibrary: PresetLibrary | null = null
  /** Library files waiting to be written or removed, by preset id, in order: the core's state is the truth until they are. */
  private presetWrites = new Set<string>()
  private presetQueue: Promise<void> = Promise.resolve()
  /** Counts the library writes queued: a read of the library that saw it move is stale. */
  private presetQueued = 0
  /** Told to the other tabs of this browser when the library changed, so that they read it again; set by `createStores`. */
  presetChannel: BroadcastChannel | null = null

  /**
   * Reads the library from the browser's storage and gives it to the core (`pw_stage_library`). A write still
   * waiting is applied first, so that a read never takes back a preset the core already holds. A file that does
   * not read is left where it is (the console names it).
   */
  async loadLibrary(): Promise<void> {
    const library = this.presetLibrary
    const core = this.core
    if (!library || !core) return
    // A write that started or ended while the files were read makes the listing stale: read again once the
    // writes are done (they are on disk then), a few times at most; a write that keeps failing waits for its retry.
    for (let attempt = 0; attempt < 4; attempt++) {
      await this.presetQueue
      if (this.presetWrites.size > 0 || this.core !== core) return
      const queued = this.presetQueued
      const files = await library.read()
      if (this.presetQueued !== queued || this.presetWrites.size > 0 || this.core !== core) continue
      const report = core.stageLibrary(files)
      if (report.skipped.length > 0) console.warn('presets left out of the library', report.skipped)
      this.project.presetsRev++
      return
    }
  }

  /** Writes or removes the file of each library preset the events name. */
  private writePresets(events: readonly ProjectEvent[]) {
    for (const event of events) {
      if (event.type !== 'preset_changed') continue
      this.project.presetsRev++
      if (event.location === 'library') this.writePreset(event.preset)
    }
  }

  /** Queues the write of a library preset's file, or its removal when the core no longer holds it. */
  private writePreset(id: string) {
    const library = this.presetLibrary
    if (!library) return
    this.presetWrites.add(id)
    this.presetQueued++
    this.presetQueue = this.presetQueue.then(async () => {
      try {
        const bytes = this.core?.presetFile('library', id) ?? null
        if (bytes) await library.write(id, bytes)
        else await library.remove(id)
        this.presetWrites.delete(id)
        this.presetChannel?.postMessage({ changed: id })
        if (this.presetWrites.size === 0 && this.state.notice === 'presets.writeFailed') this.state.notice = null
      } catch (error) {
        console.error(error)
        // Another notice (the helper lost, an audio error) is not hidden by this one.
        if (this.state.notice === null) this.state.notice = 'presets.writeFailed'
        // Kept pending: the next try (the page coming back, another tab's change) writes it again.
      }
    })
  }

  /** Writes again what could not be written. */
  retryPresets() {
    for (const id of [...this.presetWrites]) this.writePreset(id)
  }

  /** `preset.list` for a device kind (the device browser's list, and the checks). `null` before the core exists. */
  listPresetsOfKind(kind: string): PresetList | null {
    return this.presetList(listPresetsCommand({ kind }))
  }

  private presetList(command: Command): PresetList | null {
    const core = this.core
    if (!core) return null
    const { report } = core.run(command)
    if (report.status !== 'applied' || !('presets' in report.output)) return null
    return { presets: report.output.presets, current: report.output.current ?? null, modified: report.output.modified ?? null }
  }

  /** `preset.list` for a device: what its panel shows. `null` before the core exists. */
  listPresets(device: string): PresetList | null {
    return this.presetList(listPresetsCommand({ device }))
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
    // A paired page links for the agents whatever plays (#608).
    if (found === 'available') void this.linkControl()
    return found
  }

  /** The page's control link to the helper (#608): the agents' relay, in Web mode as in Boost. */
  private control: ControlLink | null = null
  /** The control link being opened: a second request joins it. */
  private controlOpening: Promise<void> | null = null
  /** The next probe after the control link was lost or could not open; `null` while none waits. */
  private controlTimer: ReturnType<typeof setTimeout> | null = null
  /** How many probes in a row found no link to open since the last one was lost (`CONTROL_RETRY_MS`). */
  private controlRetries = 0

  /**
   * Opens the control link with the stored pairing token, unless one is open with it (#608): the helper relays the
   * agents' calls there when no Boost session is open, so a paired page serves them in Web mode and when the helper has
   * no audio output. A link the helper refuses while it runs means the token is no longer its own: the pairing is
   * forgotten, as for a refused session. A link that could not open, or is lost later, is tried again (`retryControl`).
   */
  async linkControl(): Promise<void> {
    while (this.controlOpening) await this.controlOpening
    const token = storage()?.getItem(TOKEN_KEY)
    if (!token) return this.unlinkControl()
    if (this.control?.token === token) return
    this.control?.close()
    this.control = null
    // Another token: maybe another helper, which never heard of this one's agents.
    this.linkLost()
    const opening = this.openControl(token).finally(() => {
      if (this.controlOpening === opening) this.controlOpening = null
    })
    this.controlOpening = opening
    await opening
  }

  private async openControl(token: string) {
    let link: ControlLink
    try {
      link = await ControlLink.open(token, {
        onAgentCall: (agent, command, answer, signal) => void answerAgentCall(this, agent, command, signal).then(answer),
        onAgentLeft: (agent) => this.agentLeft(agent),
        onClosed: (pushedOut) => {
          if (this.control !== link) return
          this.control = null
          this.linkLost()
          // Pushed out by newer pages' links: coming back would push another out in turn, so the page waits for
          // its next probe, pairing or Boost.
          if (!pushedOut) this.retryControl()
        },
      })
    } catch (error) {
      // Too slow to open: a busy helper keeps the pairing, the link is tried again.
      if (error instanceof ControlTimeout) return this.retryControl()
      // Another protocol than the page's (#658): shown as a probe shows it; no retry, the next probe decides.
      if (error instanceof ControlIncompatible) {
        if (!this.native) {
          this.state.helper = 'incompatible'
          this.state.helperProtocol = error.protocol
        }
        return
      }
      // Refused while the helper answers its probe: the token is not this helper's (a reinstall). A pairing that
      // happened meanwhile stored a newer token: that one stays.
      const probe = await probeHelper()
      if (probe.status === 'available' && storage()?.getItem(TOKEN_KEY) === token) this.forgetPairing()
      else this.retryControl()
      return
    }
    // Forgotten, or paired again, while it opened.
    if (storage()?.getItem(TOKEN_KEY) !== token) {
      link.close()
      return void this.linkControl()
    }
    this.control = link
    this.controlRetries = 0
    if (this.controlTimer) clearTimeout(this.controlTimer)
    this.controlTimer = null
  }

  /** Probes the helper again after the next delay of `CONTROL_RETRY_MS`; a helper found there brings the link back. */
  private retryControl() {
    if (this.controlTimer || !storage()?.getItem(TOKEN_KEY)) return
    const delay = CONTROL_RETRY_MS[Math.min(this.controlRetries, CONTROL_RETRY_MS.length - 1)]
    this.controlRetries++
    this.controlTimer = setTimeout(() => {
      this.controlTimer = null
      if (this.control || !storage()?.getItem(TOKEN_KEY)) return
      // Its own probe first, so the helper's state is not shown as `checking` at each try. A helper found is
      // shown by `checkHelper`, and linked here too: in Boost `checkHelper` leaves the state, and the link, alone.
      void probeHelper().then((probe) => {
        if (this.control) return
        if (probe.status !== 'available') return this.retryControl()
        if (!this.native) void this.checkHelper()
        void this.linkControl()
      })
    }, delay)
  }

  /** Closes the control link and stops trying it again (the pairing is gone). */
  private unlinkControl() {
    if (this.controlTimer) clearTimeout(this.controlTimer)
    this.controlTimer = null
    this.controlRetries = 0
    const link = this.control
    this.control = null
    link?.close()
    this.linkLost()
  }

  /**
   * The agent `agent` left (#641, the helper's `agent.left`): the history group it left open is closed and its turn
   * ends, so the user can undo at once, and its questions to the user are withdrawn: nothing runs for an agent gone.
   */
  private agentLeft(agent: string) {
    const author = agentAuthor(agent)
    this.core?.agentLeft(author)
    this.confirmations.withdraw(author)
  }

  /**
   * A link to the helper ended, whoever ended it (#641). With neither the control link nor a Boost session left, no agent
   * can reach this page, and none can be heard leaving: the group an agent left open is closed and any agent's turn ends,
   * or a helper restarted meanwhile (which never knew the agents of the one before) would leave the user without undo.
   * Their questions were withdrawn with their links.
   */
  private linkLost() {
    if (!this.control && !this.native) this.core?.agentsGone()
  }

  /** The page came to the front (shown or focused): agents' calls come to it while no Boost session is open (#608). */
  toFront() {
    if (this.control) return this.control.front()
    // No link (pushed out by newer pages', or lost and not back yet): the page the user turns to links again, as the
    // newest, so the agents reach it. Only on the user's turn to it, so pages never push each other out in a loop.
    if (storage()?.getItem(TOKEN_KEY)) void this.linkControl()
  }

  /** The helper refused this page's token while it runs: the pairing is forgotten, the link closed. */
  private forgetPairing() {
    storage()?.removeItem(TOKEN_KEY)
    this.state.paired = false
    this.unlinkControl()
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
      // The agents reach the page from now on, whether Boost opens or not (#608).
      void this.linkControl()
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
      // The page's control link stays through Boost and after it (#608): the session takes the agents' calls first.
      void this.linkControl()
      this.state.selectedInput = null
      // The helper's engine starts from the page's project before anything is awaited: no change reaches
      // it before its snapshot (BO-1). A snapshot that does not encode leaves it nothing to play.
      this.nativeResyncs = []
      if (!this.syncNative(session, session.epoch)) throw new Error('the project snapshot for the helper did not encode')
      // The helper's engine reads the project's audio files from its folder (AU-7).
      this.state.helperFolder = null
      this.state.helperLinkFailed = false
      void this.tellHelperPlace()
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
      if ((await this.checkHelper()) === 'available' && storage()?.getItem(TOKEN_KEY) === token) this.forgetPairing()
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
    this.helperExportLost()
    this.linkLost()
    this.boostSince = null
    this.state.mode = 'web'
    this.state.helperUpdate = null
    this.state.helperFolder = null
    this.state.helperLinkFailed = false
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
