// The page's command → engine stream mapping, in Rust (crates/project-wasm over
// commands::engine_msgs, PR-3): the module runs each command on its one-synth project and
// returns the engine messages of the outcome. TypeScript never maps a command to the engine.
// Interim C ABI until PR-4.
import type { Command } from '../generated/protocol'
import projectUrl from './wasm/project.wasm?url'

interface ProjectExports {
  memory: WebAssembly.Memory
  pw_inbox(len: number): number
  pw_out(): number
  pw_stream_new(epoch: number): number
  pw_stream_free(stream: number): void
  pw_setup(stream: number): number
  pw_command(stream: number, len: number): number
}

/** The command JSON is not a command this build knows (`ERR_BAD_COMMAND`). */
export const ERR_BAD_COMMAND = -100
/** The project refused the command (unknown track or device, invalid value): nothing changed. */
export const ERR_REFUSED = -101

let loading: Promise<WebAssembly.Module> | null = null

/** Compiles the module once per page. */
export function projectModule(bytes?: BufferSource): Promise<WebAssembly.Module> {
  if (bytes) return WebAssembly.compile(bytes)
  loading ??= fetch(projectUrl)
    .then((r) => r.arrayBuffer())
    .then((b) => WebAssembly.compile(b))
  return loading
}

/** One engine channel's stream: its epoch and sequence numbers live in Rust. */
export class EngineStream {
  private readonly exports: ProjectExports
  private readonly stream: number
  private readonly encoder = new TextEncoder()

  constructor(module: WebAssembly.Module, epoch: number) {
    this.exports = new WebAssembly.Instance(module, {}).exports as unknown as ProjectExports
    this.stream = this.exports.pw_stream_new(epoch)
  }

  private take(len: number): Uint8Array {
    if (len < 0) throw new Error(`engine stream: error ${len}`)
    return new Uint8Array(this.exports.memory.buffer, this.exports.pw_out(), len).slice()
  }

  /** The setup of a fresh engine: a `Reset` to this epoch and the snapshot of the project. */
  setup(): Uint8Array {
    return this.take(this.exports.pw_setup(this.stream))
  }

  /**
   * Runs a command on the project and returns its engine messages; empty when it has none, or
   * when the project refused it (a warning: the dispatcher only sends commands the project
   * takes, so a refusal is a page bug, not a user error).
   */
  command(command: Command): Uint8Array {
    const json = this.encoder.encode(JSON.stringify(command))
    const inbox = this.exports.pw_inbox(json.length)
    if (!inbox) throw new Error('engine stream: out of memory')
    new Uint8Array(this.exports.memory.buffer, inbox, json.length).set(json)
    const len = this.exports.pw_command(this.stream, json.length)
    if (len === ERR_REFUSED) {
      console.warn(`the project refused ${command.type}`)
      return new Uint8Array()
    }
    return this.take(len)
  }
}
