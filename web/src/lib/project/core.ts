// The page's project core (PR-4): crates/project-wasm, the project, its session and history
// and the commands, compiled to WebAssembly for the main thread. The page owns the project in
// both modes (spec § 2.1): every command runs here and comes back as a report (output,
// events) with the engine bytes it produced; TypeScript never maps a command to the engine.
// The report types are generated from the Rust schema (`pnpm gen:protocol`).
import type {
  Command,
  LoadReport,
  ProjectChanges,
  ProjectEvent,
  ProjectFiles,
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
  pw_load(core: number, len: number): number
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

  /** Replaces the project with the one in `files` (empty history, snapshot in the next epoch). */
  load(files: ProjectFiles): WithEngine<LoadReport> {
    const len = this.input(JSON.stringify(files))
    const report = this.output<LoadReport>('load', this.exports.pw_load(this.live(), len))
    return { report, engine: this.engine() }
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
