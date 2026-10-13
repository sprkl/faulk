import type { Command } from '../generated/protocol'
import type { Engine, EngineEvents, EngineStatus } from './engine'
import { commandToOp } from './wasm-ops'
import type { WorkletMessage } from './worklet'
import workletUrl from './worklet.ts?worker&url'
import wasmUrl from './wasm/engine.wasm?url'

type FromWorklet =
  | { type: 'ready' }
  | { type: 'error'; message: string }
  | { type: 'meter'; peak: number; voices: number }

/** The engine compiled to WebAssembly, running in an AudioWorklet. */
export class WebEngine implements Engine {
  readonly kind = 'web' as const

  private constructor(
    private readonly context: AudioContext,
    private readonly node: AudioWorkletNode,
  ) {}

  /** Must be called from a user gesture (browsers start audio only then). */
  static async create(events: EngineEvents): Promise<WebEngine> {
    const context = new AudioContext({ latencyHint: 'interactive' })
    const [bytes] = await Promise.all([
      fetch(wasmUrl).then((r) => r.arrayBuffer()),
      context.audioWorklet.addModule(workletUrl),
    ])
    const node = new AudioWorkletNode(context, 'faulk-engine', {
      numberOfInputs: 0,
      outputChannelCount: [2],
    })
    const ready = new Promise<void>((resolve, reject) => {
      node.port.onmessage = (event: MessageEvent<FromWorklet>) => {
        const data = event.data
        if (data.type === 'ready') resolve()
        else if (data.type === 'error') reject(new Error(`audio worklet: ${data.message}`))
        else events.onMeter(data.peak, data.voices)
      }
    })
    node.port.postMessage({ type: 'init', bytes } satisfies WorkletMessage, [bytes])
    await ready
    node.connect(context.destination)
    await context.resume()
    const engine = new WebEngine(context, node)
    events.onStatus(engine.status())
    // outputLatency is only known once audio runs: report again shortly after.
    setTimeout(() => events.onStatus(engine.status()), 500)
    return engine
  }

  status(): EngineStatus {
    const seconds = this.context.baseLatency + (this.context.outputLatency || 0)
    return {
      kind: this.kind,
      sampleRate: this.context.sampleRate,
      estimatedLatencyMs: seconds > 0 ? seconds * 1000 : null,
    }
  }

  run(command: Command) {
    const op = commandToOp(command)
    if (op) this.node.port.postMessage({ type: 'ops', ops: [op] } satisfies WorkletMessage)
  }

  /** Pauses audio without losing the engine (used while the helper plays). */
  async suspend() {
    this.run({ type: 'note.all_off' })
    await this.context.suspend()
  }

  async resume() {
    await this.context.resume()
  }

  async close() {
    this.run({ type: 'note.all_off' })
    await this.context.close()
  }
}
