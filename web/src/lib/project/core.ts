// The page's project core (PR-4): crates/project-wasm, the project, its session and history
// and the commands, compiled to WebAssembly for the main thread. The page owns the project in
// both modes (spec § 2.1): every command runs here and comes back as a report (output,
// events) with the engine bytes it produced; TypeScript never maps a command to the engine.
// The report types are generated from the Rust schema (`pnpm gen:protocol`).
import type {
  Command,
  ProjectChanges,
  ProjectEvent,
  ProjectView,
  RunReport,
  SaveReport,
  SnapshotReport,
} from '../generated/protocol'
import projectUrl from './wasm/project.wasm?url'

interface CoreExports {
  memory: WebAssembly.Memory
  pw_inbox(len: number): number
  pw_out(): number
  pw_engine(): number
  pw_engine_len(): number
  pw_new(kind: number, epoch: number, timeMs: number, r0: number, r1: number, r2: number): number
  pw_free(core: number): void
  pw_run(core: number, len: number): number
  pw_setup(core: number): number
  pw_snapshot(core: number): number
  pw_project(core: number): number
  pw_stage(core: number, len: number): number
  pw_save(core: number): number
  pw_saved(core: number, len: number): number
}

/** Which project a new core starts with. */
export type CoreKind = 'empty' | 'single_synth'
const KINDS: Record<CoreKind, number> = { empty: 0, single_synth: 1 }

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

/** What running a command means for the engine. */
export type EngineStep =
  /** Bytes to push (possibly empty). */
  | { push: Uint8Array; events: ProjectEvent[] }
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
  return { push: engine, events: report.events }
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
  ) {
    this.core = exports.pw_new(KINDS[kind], epoch, seed.timeMs, ...seed.random)
    if (!this.core) throw new CoreError('new', 0)
  }

  /** A core in its own instance of `module`, whose engine stream starts in `epoch` (≥ 1). */
  static create(module: WebAssembly.Module, kind: CoreKind, epoch: number, seed: Seed = pageSeed()): ProjectCore {
    const exports = new WebAssembly.Instance(module, {}).exports as unknown as CoreExports
    return new ProjectCore(exports, kind, epoch, seed)
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

  /** Runs a command (history included): its report and its engine bytes. */
  run(command: Command): WithEngine<RunReport> {
    const len = this.input(JSON.stringify(command))
    const report = this.output<RunReport>('run', this.exports.pw_run(this.live(), len))
    return { report, engine: this.engine() }
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

  /** The project and the session as they are now. */
  project(): ProjectView {
    return this.output<ProjectView>('project', this.exports.pw_project(this.live()))
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

  /** The changes since the last recorded write; call `saved` once they are written. */
  save(): SaveReport {
    return this.output<SaveReport>('save', this.exports.pw_save(this.live()))
  }

  /** Records the changes written to the folder, as `save` gave them. */
  saved(changes: ProjectChanges) {
    const len = this.input(JSON.stringify(changes))
    const code = this.exports.pw_saved(this.live(), len)
    if (code !== 0) throw new CoreError('saved', code)
  }

  /** Frees the core; any later call throws. */
  free() {
    if (this.core) this.exports.pw_free(this.core)
    this.core = 0
  }
}
