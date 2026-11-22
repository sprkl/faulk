// Exporting the song as a WAV file (issue #297): the dialog's state and the run of one export.
//
// The render is `engine::offline` in a Worker of its own (`export-worker.ts`): the same engine
// code that plays the project, fed the stream the project core encodes, run faster than real
// time. This controller asks the core for the job, asks the user where the file goes (that needs
// the click that opened the dialog, so it comes before the render), moves the Worker's bytes into
// the file as they are made, and shows progress. Cancelling terminates the Worker and drops the
// file. A browser without the File System Access API gets a download at the end.
//
// In Boost (#319), once the helper has the project's folder open, the helper renders the same job natively and
// writes the file into that folder (`exports/<name>.wav`): no file dialog, the page sends the job and shows the
// helper's progress. A job too large to send, or no folder the helper holds, renders in the page as in Web mode.
import type { HelperExportMessage } from '../engine/engine'
import { EXPORT_ERROR_CODES, EXPORT_PROGRESS_MS, MAX_EXPORT_STREAM_BYTES, type Command } from '../generated/protocol'
import type { MessageKey } from '../i18n'
import { exportAudio } from '../project-commands'
import type { ExportJob, ExportRequest } from '../project/core'
import type { FolderHandle } from '../storage/types'
import type { FromExport, ToExport } from './export-render'
import { helperFileName, toBase64, type HelperExportPort } from './helper-export'

/** The settings the dialog offers. */
export interface ExportChoices {
  region: 'song' | 'loop' | 'bars'
  bitDepth: 'pcm16' | 'pcm24' | 'float32'
  sampleRate: 44_100 | 48_000 | 96_000
  /** The first and the last bar (counted from 1, the last one included) of the `bars` region. */
  fromBar: number
  toBar: number
  /**
   * The file's name as the dialog's field holds it, without `.wav` (#627). It starts as the project's name each time the
   * dialog opens; a project that is renamed meanwhile does not change what the user typed.
   */
  fileName: string
}

export const DEFAULT_CHOICES: ExportChoices = { region: 'song', bitDepth: 'pcm24', sampleRate: 48_000, fromBar: 1, toBar: 1, fileName: '' }

/**
 * The most a download may weigh: the browser that has no file dialog keeps the whole file in memory
 * until the end (a file in a folder is written as it is made).
 */
export const MAX_DOWNLOAD_BYTES = 400_000_000

export const BITS = { pcm16: 16, pcm24: 24, float32: 32 } as const

/** Where the file's bytes go. */
export interface ExportSink {
  /** The file's name, as the user chose it (or as the download is named). */
  readonly name: string
  /** `file`: written where the user chose; `download`: handed to the browser at the end. */
  readonly kind: 'file' | 'download'
  write(bytes: Uint8Array): Promise<void>
  /** Puts the header at the start of the file and completes it. */
  finish(header: Uint8Array): Promise<void>
  /** Drops what was written. */
  abort(): Promise<void>
}

export type SinkChoice = { status: 'chosen'; sink: ExportSink } | { status: 'cancelled' } | { status: 'refused' }

/** The Worker, as the controller drives it (a stand-in in the tests). */
export interface ExportWorkerPort {
  postMessage(message: ToExport, transfer?: Transferable[]): void
  onmessage: ((event: MessageEvent<FromExport>) => void) | null
  onerror: ((event: ErrorEvent) => void) | null
  terminate(): void
}

/** The browser calls an export makes, so tests run it without a dialog or a Worker. */
export interface ExportDeps {
  /** Asks where the file goes; `headerBytes` is room to keep at its start. Call from a click. */
  pick(name: string, headerBytes: number, startIn: FolderHandle | null): Promise<SinkChoice>
  spawn(): ExportWorkerPort
  /** The engine module's bytes (a copy: the Worker takes it). */
  engineModule(): Promise<ArrayBuffer>
  /** Whether the browser can write where the user chooses (else the file is downloaded). */
  readonly canChoose: boolean
  now(): number
}

/** What the export needs of the page's project: the dispatcher implements it. */
export interface ExportHost {
  /** Runs a command through the dispatcher, the one entry point of every action. */
  run(command: Command): void
  /** How many bars the arrangement holds (at least 1): where the bars of the dialog end at first. */
  songBars(): number
  /** The render's job for these settings; `null` while there is no project core. */
  job(request: ExportRequest): ExportJob | null
  projectName(): string
  /** Whether a clip is placed in the arrangement (an empty one has nothing to render). */
  hasContent(): boolean
  /** Whether a loop region is set. */
  hasLoop(): boolean
  /** The project's folder, where the file dialog opens. */
  folder(): Promise<FolderHandle | null>
  /** The folder the engine reads the audio clips' files from (the same as the live engine's; #647), or none. */
  mediaFolder(): FileSystemDirectoryHandle | null
  /** Whether an agent's analysis (`project.analyze_render`) renders now: two renders at once would double the memory. */
  analysisRunning(): boolean
  /** The names of these media (blob hashes) as the project calls them; a hash it does not know stays as it is. */
  mediaNames(hashes: string[]): string[]
  /**
   * The helper's export (#319): in Boost, with a helper that exports and holds the project's folder open, else
   * `null` (the page renders and the browser asks where the file goes).
   */
  helperExport(): HelperExportPort | null
  /** The project's id, which the helper checks against its folder's. */
  projectId(): string
}

/** What a finished export tells the user. */
export interface ExportResult {
  /** The file's name; for `folder`, its path in the project's folder (`exports/<name>.wav`). */
  name: string
  /** `folder`: written by the helper into the project's folder (Boost, #319). */
  kind: 'file' | 'download' | 'folder'
  bytes: number
  seconds: number
  /** The highest sample, in dBFS (`-Infinity` for silence). */
  peakDb: number
  /** Samples an integer format clipped. Always 0 now the output protection holds the ceiling (#402). */
  clipped: number
  /**
   * How deep the output protection attenuated the mix at its worst, in dB (0 when it never
   * engaged; #402). Above {@link LIMITER_PUSHED_DB} the dialog warns the mix was pushed into
   * the limiter: the file does not clip, but it was audibly limited, so the mix should be turned
   * down.
   */
  reductionDb: number
  /**
   * Frames the output protection silenced because a sample was not finite (a NaN or an infinity
   * from a device or the mix; #402): 0 on a clean render. Not shown to the user; read by the checks.
   */
  silencedFrames: number
  /** An effect still rang after the longest tail: the file stops there. */
  tailCut: boolean
  /** The audio files that could not be read, by name: their clips are silent in the file (#647). */
  missing: string[]
  /** How long the render took, in ms. */
  renderMs: number
}

/**
 * The gain reduction, in dB, above which the export warns the mix was pushed into the limiter.
 * Below it the limiter only catches the odd peak, which is inaudible and not worth a warning.
 */
export const LIMITER_PUSHED_DB = 1

/**
 * The most gain reduction the warning reports, in dB. A finite runaway (a device blowing up to a
 * large but finite level) can bring the deficit to ~1.0, i.e. an unbounded dB figure; past this
 * the exact number means nothing to the user, so the text caps at "≥ {MAX_REPORTED_REDUCTION_DB}".
 */
export const MAX_REPORTED_REDUCTION_DB = 60

export type ExportPhase = 'options' | 'picking' | 'rendering' | 'done' | 'failed'

/** A file name for the project: nothing a file system refuses. */
export function fileNameOf(project: string): string {
  return `${cleanName(project.trim().replace(/\.wav$/i, ''))}.wav`
}

/** What a file name keeps of a name: no character a file system refuses, nothing but a name; `Faulk` when nothing is left. */
export function cleanName(project: string): string {
  const cleaned = project
    // eslint-disable-next-line no-control-regex
    .replace(/[\\/:*?"<>|\u0000-\u001f]/g, '-')
    .replace(/^[\s.]+|[\s.]+$/g, '')
    .slice(0, 100)
  return cleaned || 'Faulk'
}

export class AudioExport {
  /** Whether the dialog is showing. */
  open = $state(false)
  phase = $state<ExportPhase>('options')
  choices = $state<ExportChoices>({ ...DEFAULT_CHOICES })
  /** How far the content is rendered, 0 to 1. */
  fraction = $state(0)
  /** The content is rendered; the effects are ringing out. */
  tail = $state(false)
  result = $state.raw<ExportResult | null>(null)
  error = $state<MessageKey | null>(null)

  /** The Export button ran the command and waits for it to come back as an effect (see `request`). */
  private confirming = false
  private worker: ExportWorkerPort | null = null
  private sink: ExportSink | null = null
  private writing: Promise<void> = Promise.resolve()
  /** Counts runs, so a late message of a cancelled one never reaches the next. */
  private run = 0
  /** The helper's export under way (#319): its port, its number, and the end of its listening. */
  private helper: { port: HelperExportPort; id: number; unlisten: () => void } | null = null

  constructor(
    private readonly host: ExportHost,
    private readonly deps: ExportDeps,
  ) {}

  /** Whether the browser asks where the file goes (else it goes to the downloads). */
  get canChoose(): boolean {
    return this.deps.canChoose
  }

  /** Whether the helper writes the file into the project's `exports/` folder (Boost, #319). */
  get inFolder(): boolean {
    return this.host.helperExport() !== null
  }

  get hasContent(): boolean {
    return this.host.hasContent()
  }

  get hasLoop(): boolean {
    return this.host.hasLoop()
  }

  get busy(): boolean {
    return this.phase === 'picking' || this.phase === 'rendering'
  }

  /** The bars of the dialog make a range (whole numbers, from 1, the last one not before the first). */
  get barsValid(): boolean {
    const { fromBar, toBar } = this.choices
    return Number.isInteger(fromBar) && Number.isInteger(toBar) && fromBar >= 1 && toBar >= fromBar
  }

  /** Whether the dialog's settings can be exported. */
  get canStart(): boolean {
    return this.hasContent && (this.choices.region !== 'bars' || this.barsValid)
  }

  /** Opens the dialog (the menu); `choices` fills in what a command asked for. */
  show(choices: Partial<ExportChoices> = {}) {
    if (this.busy) {
      this.open = true
      return
    }
    // A setting a command left out keeps its default.
    this.choices = {
      ...DEFAULT_CHOICES,
      toBar: this.host.songBars(),
      fileName: cleanName(this.host.projectName()),
      ...Object.fromEntries(Object.entries(choices).filter(([, value]) => value !== undefined)),
    }
    // A loop that does not exist is not offered.
    if (this.choices.region === 'loop' && !this.hasLoop) this.choices.region = 'song'
    this.phase = 'options'
    this.result = null
    this.error = null
    this.open = true
  }

  /**
   * `project.export_audio` ran and the project accepted it. Run by the Export button
   * (`start`) it begins the render; run by anyone else (an agent), it opens the dialog with its
   * settings, because the file dialog of a browser needs the click of a user.
   */
  request(request: ExportRequest) {
    if (this.confirming) {
      this.confirming = false
      void this.begin()
      return
    }
    const rate = request.sample_rate
    this.show({
      region: request.region,
      bitDepth: request.bit_depth,
      sampleRate: rate === 44_100 || rate === 96_000 ? rate : 48_000,
      fromBar: request.from_bar ?? undefined,
      toBar: request.to_bar ?? undefined,
    })
  }

  /** Closes the dialog. A render under way is cancelled. */
  close() {
    if (this.busy) this.cancel()
    this.open = false
  }

  /** Stops the render and drops the file. */
  cancel() {
    if (!this.busy) return
    this.stop()
    this.phase = 'options'
    this.fraction = 0
    this.tail = false
  }

  private stop() {
    // Whatever the stopped run still says changes nothing.
    this.run++
    this.worker?.terminate()
    this.worker = null
    const helper = this.helper
    this.helper = null
    if (helper) {
      helper.unlisten()
      helper.port.cancel(helper.id)
    }
    const sink = this.sink
    this.sink = null
    if (sink) void this.writing.then(() => sink.abort()).catch(() => undefined)
    this.writing = Promise.resolve()
  }

  private fail(key: MessageKey, detail?: unknown) {
    if (detail !== undefined) console.error('export:', detail)
    this.stop()
    this.phase = 'failed'
    this.error = key
  }

  /**
   * The Export button: runs `project.export_audio` with the dialog's settings, which the project
   * checks (and, accepted, hands back to `request`, which begins). Call straight from the click:
   * the file dialog of the browser needs it, and it opens right after the project's folder is known
   * (at once, unless a move of the project into a folder is under way).
   */
  start() {
    if (this.busy) return
    if (this.host.analysisRunning()) {
      this.fail('export.failed', 'an analysis is rendering: try again when it is done')
      return
    }
    this.confirming = true
    this.host.run(exportAudio(this.request_()))
    // The project refused it: nothing came back. Why, the core says again.
    if (this.confirming) {
      this.confirming = false
      const job = this.host.job(this.request_())
      this.fail(job?.status === 'refused' && job.error.code === 'limit' ? 'export.tooLong' : 'export.refused')
    }
  }

  /** The dialog's settings as the command takes them. */
  private request_(): ExportRequest {
    const bars = this.choices.region === 'bars' ? { from_bar: this.choices.fromBar, to_bar: this.choices.toBar } : {}
    return {
      region: this.choices.region,
      bit_depth: this.choices.bitDepth,
      sample_rate: this.choices.sampleRate,
      ...bars,
    }
  }

  /** Asks the core for the render, asks for the file, renders. */
  private async begin() {
    const run = ++this.run
    const job = this.host.job(this.request_())
    if (!job) return this.fail('export.failed')
    if (job.status !== 'ready') {
      const tooLong = job.status === 'refused' && job.error.code === 'limit'
      return this.fail(job.status === 'refused' ? (tooLong ? 'export.tooLong' : 'export.refused') : 'export.failed', job)
    }
    // Boost, the folder open in the helper, a job small enough to send: the helper renders and writes (#319).
    const port = this.host.helperExport()
    if (port && job.setup.length + job.start.length + job.stop.length <= MAX_EXPORT_STREAM_BYTES) return this.inHelper(run, job, port)
    // Without a file dialog the whole file waits in memory until the end: a long one does not fit.
    const frameBytes = (2 * BITS[this.choices.bitDepth]) / 8
    if (!this.deps.canChoose && job.plan.body_frames * frameBytes > MAX_DOWNLOAD_BYTES) return this.fail('export.tooBig')
    this.error = null
    this.result = null
    this.fraction = 0
    this.tail = false
    this.phase = 'picking'
    const headerBytes = headerLength(this.choices.bitDepth)
    // First: the browser lets a page ask for a file only straight after a click.
    // The folder waits for a save under way; a cancel in the meantime must not open the dialog afterwards.
    const folder = await this.host.folder()
    if (run !== this.run) return
    const choice = await this.deps.pick(fileNameOf(this.choices.fileName), headerBytes, folder)
    if (run !== this.run) {
      if (choice.status === 'chosen') void choice.sink.abort().catch(() => undefined)
      return
    }
    if (choice.status === 'cancelled') {
      this.phase = 'options'
      return
    }
    if (choice.status === 'refused') return this.fail('export.noAccess')
    const sink = choice.sink
    this.sink = sink
    let module: ArrayBuffer
    try {
      module = await this.deps.engineModule()
    } catch (error) {
      if (run === this.run) this.fail('export.failed', error)
      return
    }
    if (run !== this.run) return
    this.phase = 'rendering'
    const started = this.deps.now()
    const worker = this.deps.spawn()
    this.worker = worker
    let bytes = headerBytes
    const body = job.plan.body_frames
    const rate = this.choices.sampleRate
    worker.onerror = (event) => run === this.run && this.fail('export.failed', event.message)
    worker.onmessage = (event) => {
      if (run !== this.run) return
      const message = event.data
      if (message.type === 'chunk') {
        this.fraction = Math.min(1, message.rendered / Math.max(1, message.body))
        this.tail = message.tail
        if (message.bytes.byteLength > 0) {
          const chunk = new Uint8Array(message.bytes)
          bytes += chunk.length
          // The Worker may send more once this chunk is written: what waits in memory stays bounded.
          this.writing = this.writing.then(async () => {
            if (run !== this.run) return
            await sink.write(chunk)
            if (run === this.run) worker.postMessage({ type: 'ack' })
          })
          // A failed write (a full disk, a revoked permission) ends the export; the chain keeps the error.
          this.writing = this.writing.catch((error: unknown) => {
            if (run === this.run) this.fail('export.writeFailed', error)
          })
        }
      } else if (message.type === 'done') {
        const header = new Uint8Array(message.header)
        void this.writing
          .then(() => (run === this.run ? sink.finish(header) : undefined))
          .then(() => {
            if (run !== this.run) return
            this.worker?.terminate()
            this.worker = null
            this.sink = null
            this.fraction = 1
            this.result = {
              name: sink.name,
              kind: sink.kind,
              bytes,
              seconds: message.written / rate,
              peakDb: message.peak > 0 ? 20 * Math.log10(message.peak) : -Infinity,
              clipped: message.clipped,
              // 20·log10(1 - reduction): the deepest attenuation. reduction is in [0, 1); near 1
              // (a finite runaway) the log diverges, so cap the reported figure at a value past
              // which the exact number is meaningless to the user.
              reductionDb: message.reduction > 0 ? Math.min(-20 * Math.log10(Math.max(1 - message.reduction, 1e-9)), MAX_REPORTED_REDUCTION_DB) : 0,
              silencedFrames: message.silenced,
              tailCut: message.tailCut,
              missing: this.host.mediaNames(message.missing),
              renderMs: this.deps.now() - started,
            }
            this.phase = 'done'
          })
          .catch((error: unknown) => {
            if (run === this.run) this.fail('export.writeFailed', error)
          })
      } else {
        this.fail('export.failed', `${message.message} (code ${message.code})`)
      }
    }
    const setup = job.setup.slice().buffer
    const start = job.start.slice().buffer
    const stop = job.stop.slice().buffer
    const media = this.host.mediaFolder()
    worker.postMessage({ type: 'start', module, sampleRate: rate, bits: BITS[this.choices.bitDepth], bodyFrames: body, setup, start, stop, folder: media }, [
      module,
      setup,
      start,
      stop,
    ])
  }

  /** The helper renders `job` and writes its file into the project's folder (#319); the dialog shows its progress. */
  private inHelper(run: number, job: Extract<ExportJob, { status: 'ready' }>, port: HelperExportPort) {
    this.error = null
    this.result = null
    this.fraction = 0
    this.tail = false
    this.phase = 'rendering'
    const started = this.deps.now()
    const rate = this.choices.sampleRate
    // A helper that stops answering (a message it dropped, a run that hangs) fails the export rather than leaving
    // the dialog on its progress bar: a render says how far it is at least every `EXPORT_PROGRESS_MS`.
    let watchdog: ReturnType<typeof setTimeout> | undefined
    const arm = () => {
      clearTimeout(watchdog)
      watchdog = setTimeout(() => {
        if (run === this.run) this.fail('export.helperSilent', 'the helper said nothing of its export')
      }, HELPER_SILENCE_MS)
    }
    const stopListening = () => {
      clearTimeout(watchdog)
      unlisten()
    }
    const unlisten = port.listen((message: HelperExportMessage | null) => {
      if (run !== this.run) return
      if (message === null) {
        // The session is gone, and its export with it (the helper cancels the export of a page that left).
        stopListening()
        this.helper = null
        return this.fail('export.helperLost')
      }
      if (message.id !== run) return
      arm()
      if (message.type === 'export.progress') {
        this.fraction = Math.min(1, message.rendered / Math.max(1, message.body))
        this.tail = message.tail
      } else if (message.type === 'export.done') {
        stopListening()
        this.helper = null
        this.fraction = 1
        this.result = {
          name: message.path,
          kind: 'folder',
          bytes: message.bytes,
          seconds: message.written / rate,
          peakDb: message.peak > 0 ? 20 * Math.log10(message.peak) : -Infinity,
          clipped: message.clipped,
          reductionDb: message.reduction > 0 ? Math.min(-20 * Math.log10(Math.max(1 - message.reduction, 1e-9)), MAX_REPORTED_REDUCTION_DB) : 0,
          silencedFrames: message.silenced,
          tailCut: message.tail_cut,
          missing: this.host.mediaNames(message.missing),
          renderMs: this.deps.now() - started,
        }
        this.phase = 'done'
      } else {
        // It ended on its own: nothing to cancel.
        stopListening()
        this.helper = null
        this.fail(helperFailure(message.code), `${message.code}: ${message.message}`)
      }
    })
    this.helper = { port, id: run, unlisten: stopListening }
    arm()
    try {
      port.start({
        id: run,
        project: this.host.projectId(),
        name: helperFileName(this.choices.fileName),
        sample_rate: rate,
        bits: BITS[this.choices.bitDepth],
        body_frames: job.plan.body_frames,
        setup: toBase64(job.setup),
        start: toBase64(job.start),
        stop: toBase64(job.stop),
      })
    } catch (error) {
      // The message could not be made (memory): nothing reached the helper.
      this.fail('export.failed', error)
    }
  }
}

/**
 * How long the page waits for a word of the helper's export before it fails it, in ms (#319): 300 of the
 * helper's progress periods, which it keeps through its render and its final write.
 */
export const HELPER_SILENCE_MS = 300 * EXPORT_PROGRESS_MS

/** The message the dialog shows for the helper's `export.failed` code (#319). */
export function helperFailure(code: string): MessageKey {
  switch (code) {
    case EXPORT_ERROR_CODES.no_folder:
    case EXPORT_ERROR_CODES.folder_changed:
      return 'export.folderChanged'
    case EXPORT_ERROR_CODES.write:
      return 'export.writeFailed'
    case EXPORT_ERROR_CODES.busy:
      return 'export.helperBusy'
    default:
      return 'export.failed'
  }
}

/** Bytes of the header of a file of this format (`engine::offline::SampleFormat::header_bytes`). */
export function headerLength(bitDepth: ExportChoices['bitDepth']): number {
  return bitDepth === 'float32' ? 58 : 44
}
