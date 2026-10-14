// The page's command → engine stream mapping, in Rust (crates/project-wasm over
// commands::engine_msgs, MO-2): TypeScript never maps a command to the engine. Interim C ABI
// until PR-4 runs the whole project in this module.
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

  /** The setup of a fresh engine: a `Reset` to this epoch, the synth's track, the synth. */
  setup(): Uint8Array {
    return this.take(this.exports.pw_setup(this.stream))
  }

  /** The bytes of a command; empty when the engine does not take it. */
  command(command: Command): Uint8Array {
    const json = this.encoder.encode(JSON.stringify(command))
    const inbox = this.exports.pw_inbox(json.length)
    if (!inbox) throw new Error('engine stream: out of memory')
    new Uint8Array(this.exports.memory.buffer, inbox, json.length).set(json)
    return this.take(this.exports.pw_command(this.stream, json.length))
  }
}
