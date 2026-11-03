// The page's project core (PR-4): crates/project-wasm, the project, its session and history
// and the commands, compiled to WebAssembly for the main thread. The page owns the project in
// both modes (spec § 2.1): every command runs here and comes back as a report (output,
// events) with the engine bytes it produced; TypeScript never maps a command to the engine.
// The report types are generated from the Rust schema (`pnpm gen:protocol`).
import type { Command, ExportReport, HistoryView, LiveRouting, ProjectEvent, ProjectView, RunReport, SnapshotReport } from '../generated/protocol'
import projectUrl from './wasm/project.wasm?url'

interface CoreExports {
  memory: WebAssembly.Memory
  pw_inbox(len: number): number
  pw_out(): number
  pw_engine(): number
  pw_engine_len(): number
  pw_new(kind: number, epoch: number, timeMs: number, r0: number, r1: number, r2: number, nameLen: number): number
  pw_free(core: number): void
  pw_run(core: number, len: number): number
  pw_setup(core: number): number
  pw_snapshot(core: number): number
  pw_snapshot_after(core: number, other: number): number
  pw_project(core: number): number
  pw_history(core: number): number
  pw_live_routing(core: number): number
  pw_set_author(core: number, len: number): number
  pw_set_clock(core: number, nowMs: number): number
  pw_export_job(core: number, len: number): number
  pw_stage(core: number, len: number): number
  pw_stage_stored(core: number, len: number): number
  pw_save(core: number, mode: number): number
  pw_saved(core: number, applied: number): number
  pw_dirty(core: number): number
  pw_project_dirty(core: number): number
  pw_pack_export(len: number): number
  pw_pack_import(len: number): number
}

/** The projects the page can open, as `project.list_stored` answers (`StoredProjects` in `crates/commands`). */
export interface StoredProjects {
  /** The folders opened recently, newest first. */
  recent: { id: string; name: string }[]
  /** The projects kept in the browser's storage, newest first. */
  browser: { dir: string; name: string }[]
}

/** A `.faulkpack` made by `ProjectCore.packExport`. */
export type PackExport = { status: 'ok'; zip: Uint8Array; files: number; missingBlobs: number } | { status: 'failed'; message: string }

/** The files of a `.faulkpack` read by `ProjectCore.packImport`, in the order of a save frame. */
export type PackImport = { status: 'ok'; files: { path: string; bytes: Uint8Array }[] } | { status: 'failed'; message: string }

/** Which project a new core starts with. */
export type CoreKind = 'empty' | 'single_synth'
const KINDS: Record<CoreKind, number> = { empty: 0, single_synth: 1 }

/** Which files a save looks at (see `ProjectCore.save`). */
export type SaveMode = 'changed' | 'forced' | 'everything' | 'copy' | 'rewrite'
const SAVE_MODES: Record<SaveMode, number> = { changed: 0, forced: 1, everything: 2, copy: 3, rewrite: 4 }

/** One thing a save asks of the folder: replace a file, or remove it. */
export type SaveEntry = { type: 'write'; path: string; bytes: Uint8Array } | { type: 'remove'; path: string }

/** What a save computed: the entries in the order to apply them, or why it could not. */
export type SaveFrame = { status: 'ok'; entries: SaveEntry[] } | { status: 'failed'; message: string }

/** Reads the frame `pw_save` returns (`crates/project-wasm/src/save.rs` documents the layout). */
export function parseSaveFrame(frame: Uint8Array): SaveFrame {
  const decoder = new TextDecoder()
  if (frame[0] !== 0) return { status: 'failed', message: decoder.decode(frame.subarray(1)) }
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength)
  const count = view.getUint32(5, true)
  const entries: SaveEntry[] = []
  let at = 9
  for (let i = 0; i < count; i++) {
    const kind = frame[at++]
    const pathLength = view.getUint32(at, true)
    at += 4
    const path = decoder.decode(frame.subarray(at, at + pathLength))
    at += pathLength
    if (kind === 1) {
      entries.push({ type: 'remove', path })
      continue
    }
    const length = Number(view.getBigUint64(at, true))
    at += 8
    // Copied out of the core's memory: the next call reuses it.
    entries.push({ type: 'write', path, bytes: frame.slice(at, at + length) })
    at += length
  }
  return { status: 'ok', entries }
}

/** The engine bytes a call produced, beside its report: empty when it produced none. */
export interface WithEngine<R> {
  report: R
  engine: Uint8Array
}

/** Seeds the ids the core creates: the page's clock and 80 random bits (spec § 2.1). */
export interface Seed {
  timeMs: number
  /** Three u32, low first; the core keeps the low 80 of their 96 bits. */
  random: [number, number, number]
}

export function pageSeed(): Seed {
  const random = crypto.getRandomValues(new Uint32Array(3))
  return { timeMs: Date.now(), random: [random[0], random[1], random[2]] }
}

let loading: Promise<WebAssembly.Module> | null = null

/** Compiles the module once per page (or from the given bytes, in tests). */
export function coreModule(bytes?: BufferSource): Promise<WebAssembly.Module> {
  if (bytes) return WebAssembly.compile(bytes)
  loading ??= fetch(projectUrl)
    .then((r) => r.arrayBuffer())
    .then((b) => WebAssembly.compile(b))
  return loading
}

/**
 * One file of a project folder, as the page stages it for `project.open`: its bytes, or for a
 * blob the project only needs counted (audio, instrument state) its size. Opening reads the
 * text files and the blobs of dense automation lanes; it only lists the other blobs, to tell
 * the project which media the folder lacks. So the memory a staged folder takes is the
 * project's, not its audio's (`StagedFs` in `crates/project-wasm`).
 */
export type FolderEntry = { path: string; bytes: Uint8Array } | { path: string; size: number }

/** The first bytes of a blob of dense automation points (`crates/project/src/file/unit.rs`). */
export const LANE_BLOB_MAGIC = 'FKAUTO01'

/**
 * Whether opening needs the bytes of the file at `path`, given its first bytes (`head`, 8 are
 * enough): everything but the blobs, and the blobs of automation lanes. The page reads this
 * much of each blob to decide, and sends the rest by size.
 */
export function needsBytes(path: string, head: Uint8Array): boolean {
  if (!path.startsWith('blobs/')) return true
  return LANE_BLOB_MAGIC.length <= head.length && LANE_BLOB_MAGIC.split('').every((c, i) => head[i] === c.charCodeAt(0))
}

/** The most bytes of a staged folder (`MAX_FOLDER_FRAME` in `crates/project-wasm`). */
export const MAX_FOLDER_FRAME = 512 << 20

/** The folder frame `pw_stage` takes: `crates/project-wasm` `parse_folder` documents the layout. */
export function folderFrame(name: string, entries: FolderEntry[]): Uint8Array {
  const encoder = new TextEncoder()
  const parts: Uint8Array[] = []
  let total = 0
  const push = (part: Uint8Array) => {
    parts.push(part)
    total += part.length
  }
  const u32 = (n: number) => {
    const part = new Uint8Array(4)
    new DataView(part.buffer).setUint32(0, n, true)
    push(part)
  }
  const u64 = (n: number) => {
    const part = new Uint8Array(8)
    new DataView(part.buffer).setBigUint64(0, BigInt(n), true)
    push(part)
  }
  const text = (value: string) => {
    const bytes = encoder.encode(value)
    u32(bytes.length)
    push(bytes)
  }
  text(name)
  for (const entry of entries) {
    text(entry.path)
    if ('bytes' in entry) {
      push(Uint8Array.of(0))
      u64(entry.bytes.length)
      push(entry.bytes)
    } else {
      push(Uint8Array.of(1))
      u64(entry.size)
    }
  }
  if (total > MAX_FOLDER_FRAME) throw new CoreError('stage', ERR_BAD_FOLDER)
  const frame = new Uint8Array(total)
  let at = 0
  for (const part of parts) {
    frame.set(part, at)
    at += part.length
  }
  return frame
}

/** `pw_stage`: the frame is malformed or too big; nothing was staged. */
export const ERR_BAD_FOLDER = -4

/** A call failed below the reports (no core, bad input, a value that does not serialize). */
export class CoreError extends Error {
  constructor(
    readonly call: string,
    readonly code: number,
  ) {
    super(`project core: ${call} failed with code ${code}`)
  }
}

/** The settings of an export (`project.export_audio`'s fields; the core fills in the defaults). */
export interface ExportRequest {
  region?: 'song' | 'loop' | 'bars'
  bit_depth?: 'pcm16' | 'pcm24' | 'float32'
  sample_rate?: number
  from_bar?: number
  to_bar?: number
}

/** An export the core prepared: its plan and the two streams of the engine that renders it. */
export type ExportJob =
  | { status: 'ready'; plan: Extract<ExportReport, { status: 'ready' }>['plan']; setup: Uint8Array; stop: Uint8Array }
  | Exclude<ExportReport, { status: 'ready' }>

/** What running a command means for the engine. */
export type EngineStep =
  /** Bytes to push (possibly empty), with the command's output. */
  | { push: Uint8Array; events: ProjectEvent[]; output: unknown }
  /**
   * The command may have changed the project but the engine did not get it: behind until a
   * snapshot. `events` is empty when the report itself was lost (a `CoreError`): refresh the
   * page's mirror of the project from `ProjectCore.project`.
   */
  | { streamError: number; events: ProjectEvent[] }
  /** The project refused the command, or it is not a command: nothing changed. */
  | { refused: RunReport }

/** Runs a command through `run` and says what to do with the engine; never throws a `CoreError`. */
export function engineStep(run: () => WithEngine<RunReport>): EngineStep {
  let result: WithEngine<RunReport>
  try {
    result = run()
  } catch (error) {
    if (error instanceof CoreError) return { streamError: error.code, events: [] }
    throw error
  }
  const { report, engine } = result
  if (report.status !== 'applied') return { refused: report }
  if (typeof report.stream_error === 'number') return { streamError: report.stream_error, events: report.events }
  return { push: engine, events: report.events, output: report.output }
}

/**
 * One project in the page with the stream of the engine that plays it. Each call returns
 * what the Rust side reported; refusals are reports, never exceptions.
 */
export class ProjectCore {
  private readonly encoder = new TextEncoder()
  private readonly decoder = new TextDecoder()
  private core: number

  private constructor(
    private readonly exports: CoreExports,
    kind: CoreKind,
    epoch: number,
    seed: Seed,
    name?: string,
  ) {
    // The name of the new project, through the inbox (#473); 0 bytes: the core's default name.
    const nameLen = name ? this.input(name) : 0
    this.core = exports.pw_new(KINDS[kind], epoch, seed.timeMs, ...seed.random, nameLen)
    if (!this.core) throw new CoreError('new', 0)
  }

  /** A core in its own instance of `module`, whose engine stream starts in `epoch` (≥ 1); `name` names its project. */
  static create(module: WebAssembly.Module, kind: CoreKind, epoch: number, seed: Seed = pageSeed(), name?: string): ProjectCore {
    const exports = new WebAssembly.Instance(module, {}).exports as unknown as CoreExports
    return new ProjectCore(exports, kind, epoch, seed, name)
  }

  private input(text: string): number {
    const bytes = this.encoder.encode(text)
    const at = this.exports.pw_inbox(bytes.length)
    if (!at) throw new CoreError('inbox', 0)
    new Uint8Array(this.exports.memory.buffer, at, bytes.length).set(bytes)
    return bytes.length
  }

  private output<R>(call: string, len: number): R {
    if (len < 0) throw new CoreError(call, len)
    const bytes = new Uint8Array(this.exports.memory.buffer, this.exports.pw_out(), len)
    return JSON.parse(this.decoder.decode(bytes)) as R
  }

  private engine(): Uint8Array {
    const len = this.exports.pw_engine_len()
    if (len === 0) return new Uint8Array()
    return new Uint8Array(this.exports.memory.buffer, this.exports.pw_engine(), len).slice()
  }

  private live(): number {
    if (!this.core) throw new CoreError('freed', 0)
    return this.core
  }

  /**
   * Runs a command (history included): its report and its engine bytes. The history stamps `author` (`user`, or
   * `agent:<name>`) and the page's clock on the entry it records. The author is set before every run, never carried
   * over: one caller's name cannot land on another's edit, whatever happened to the run before.
   */
  run(command: Command, author = 'user', now = Date.now()): WithEngine<RunReport> {
    this.exports.pw_set_clock(this.live(), now)
    const named = this.input(author)
    const code = this.exports.pw_set_author(this.live(), named)
    if (code !== 0) throw new CoreError('set_author', code)
    const len = this.input(JSON.stringify(command))
    const report = this.output<RunReport>('run', this.exports.pw_run(this.live(), len))
    return { report, engine: this.engine() }
  }

  /** The undo history as the history panel shows it: the entries of both stacks, oldest first. */
  history(): HistoryView {
    return this.output<HistoryView>('history', this.exports.pw_history(this.live()))
  }

  /**
   * Opens the folder staged under `name` (`stage`): the project, its history and its session are
   * replaced and the engine bytes start it over from a snapshot of the new project in the next
   * epoch. A refusal changes nothing.
   */
  open(name: string): WithEngine<RunReport> {
    return this.run({ type: 'project.open', folder: name })
  }

  /** A `Reset` in the current epoch and the snapshot of the project: for a fresh engine. */
  setup(): WithEngine<SnapshotReport> {
    const report = this.output<SnapshotReport>('setup', this.exports.pw_setup(this.live()))
    return { report, engine: this.engine() }
  }

  /** The same in the next epoch: for an engine switched to, or one that refused the stream. */
  snapshot(): WithEngine<SnapshotReport> {
    const report = this.output<SnapshotReport>('snapshot', this.exports.pw_snapshot(this.live()))
    return { report, engine: this.engine() }
  }

  /**
   * The same for an engine the page did not start, whose epoch is `other` (the helper's, from its
   * `hello`, BO-1): the `Reset` is in an epoch newer than both, so the helper's engine takes it.
   */
  snapshotAfter(other: number): WithEngine<SnapshotReport> {
    const report = this.output<SnapshotReport>('snapshot', this.exports.pw_snapshot_after(this.live(), other & 0xffff))
    return { report, engine: this.engine() }
  }

  /**
   * Where the live input plays now (#598): the engine slots of the selected and armed MIDI tracks, in the
   * epoch of the engine stream they belong to. In Boost the page sends it to the helper (`live.routing`),
   * which plays the notes of its own MIDI input with it. Small: read after every command in Boost.
   */
  liveRouting(): LiveRouting {
    return this.output<LiveRouting>('live_routing', this.exports.pw_live_routing(this.live()))
  }

  /** The project and the session as they are now. */
  project(): ProjectView {
    return this.output<ProjectView>('project', this.exports.pw_project(this.live()))
  }

  /**
   * Prepares the render of `project.export_audio` for `request` (`crates/commands/src/export.rs`):
   * the plan, and the streams for an engine of its own. Changes nothing in the project nor in the
   * stream of the engine that plays it.
   */
  exportJob(request: ExportRequest): ExportJob {
    const len = this.input(JSON.stringify(request))
    const report = this.output<ExportReport>('export_job', this.exports.pw_export_job(this.live(), len))
    if (report.status !== 'ready') return report
    const bytes = this.engine()
    return { status: 'ready', plan: report.plan, setup: bytes.slice(0, report.setup_len), stop: bytes.slice(report.setup_len) }
  }

  /**
   * Stages a project folder under `name` for the next `project.open` that names it. Throws a
   * `CoreError` when the entries are not a valid folder (a bad path, too big): nothing is staged.
   */
  stage(name: string, entries: FolderEntry[]) {
    const frame = folderFrame(name, entries)
    const at = this.exports.pw_inbox(frame.length)
    if (!at) throw new CoreError('inbox', 0)
    new Uint8Array(this.exports.memory.buffer, at, frame.length).set(frame)
    const code = this.exports.pw_stage(this.live(), frame.length)
    if (code !== 0) throw new CoreError('stage', code)
  }

  /**
   * Stages what the page keeps, for the next `project.list_stored` to answer with (once). Throws a
   * `CoreError` when it is not a list of stored projects: nothing is staged.
   */
  stageStored(list: StoredProjects) {
    const len = this.input(JSON.stringify(list))
    const code = this.exports.pw_stage_stored(this.live(), len)
    if (code !== 0) throw new CoreError('stage_stored', code)
  }

  /**
   * The writes that bring the project's folder up to date, computed by `project::file::save`
   * in the core (SV-2). `changed` does nothing when the project did not change since the last
   * save; `forced` looks at every file anyway (the session included); `everything` writes the
   * whole project as into an empty folder (the first write of a save as). Call `saved` once
   * the entries are applied to the folder, or with `false` when they could not be: the next
   * save then computes them again.
   */
  save(mode: SaveMode): SaveFrame {
    const len = this.exports.pw_save(this.live(), SAVE_MODES[mode])
    if (len < 0) throw new CoreError('save', len)
    return parseSaveFrame(new Uint8Array(this.exports.memory.buffer, this.exports.pw_out(), len))
  }

  /** Says whether the folder now holds what the last `save` gave. */
  saved(applied: boolean) {
    const code = this.exports.pw_saved(this.live(), applied ? 1 : 0)
    if (code !== 0) throw new CoreError('saved', code)
  }

  /** Whether the project has changes (edits, solo, selection...) that the folder does not hold. */
  dirty(): boolean {
    const code = this.exports.pw_dirty(this.live())
    if (code < 0) throw new CoreError('dirty', code)
    return code === 1
  }

  /** Puts `bytes` in the core's inbox; the stateless pack calls read it from there. */
  private inbox(bytes: Uint8Array): void {
    const at = this.exports.pw_inbox(bytes.length)
    if (!at) throw new CoreError('inbox', 0)
    new Uint8Array(this.exports.memory.buffer, at, bytes.length).set(bytes)
  }

  /** The `len` bytes the last pack call left, copied out of the core's memory. */
  private packOut(call: string, len: number): Uint8Array {
    if (len < 0) throw new CoreError(call, len)
    return new Uint8Array(this.exports.memory.buffer, this.exports.pw_out(), len).slice()
  }

  /**
   * The `.faulkpack` of a project folder given whole (every file with its bytes: `project.json`,
   * the tracks, the blobs): `project::file::export_pack` in the core, so the archive is the one every
   * host reads. A folder that is not a valid project is `failed`, with the reason. Throws a
   * `CoreError` when the folder cannot be carried into the core at all (too big).
   */
  packExport(files: { path: string; bytes: Uint8Array }[]): PackExport {
    const frame = folderFrame('pack', files)
    this.inbox(frame)
    const out = this.packOut('pack_export', this.exports.pw_pack_export(frame.length))
    if (out[0] !== 0) return { status: 'failed', message: new TextDecoder().decode(out.subarray(1)) }
    const view = new DataView(out.buffer, out.byteOffset, out.byteLength)
    return { status: 'ok', files: view.getUint32(1, true), missingBlobs: view.getUint32(5, true), zip: out.subarray(9) }
  }

  /**
   * The files of a `.faulkpack`, validated as a project folder by the strict import of the format (only
   * the format's paths, every blob checked against its name, size limits). A pack that does not pass is
   * `failed`, with the reason, and nothing was kept.
   */
  packImport(zip: Uint8Array): PackImport {
    this.inbox(zip)
    const out = this.packOut('pack_import', this.exports.pw_pack_import(zip.length))
    if (out[0] !== 0) return { status: 'failed', message: new TextDecoder().decode(out.subarray(1)) }
    const frame = parseSaveFrame(out)
    if (frame.status !== 'ok') return { status: 'failed', message: frame.message }
    const files = frame.entries.flatMap((entry) => (entry.type === 'write' ? [{ path: entry.path, bytes: entry.bytes }] : []))
    return { status: 'ok', files }
  }

  /**
   * Whether the project itself, not only its session (selection, solo, arming, MIDI input), has
   * changes the folder does not hold: a page only opened has selected its first track and has
   * nothing worth a home.
   */
  projectDirty(): boolean {
    const code = this.exports.pw_project_dirty(this.live())
    if (code < 0) throw new CoreError('project_dirty', code)
    return code === 1
  }

  /** Frees the core; any later call throws. */
  free() {
    if (this.core) this.exports.pw_free(this.core)
    this.core = 0
  }
}
