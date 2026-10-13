// Runs on the audio thread, inside an AudioWorkletGlobalScope.
import { WasmCore, type EngineExports } from './wasm-core'
import type { WasmOp } from './wasm-ops'

declare const sampleRate: number
declare function registerProcessor(name: string, processor: unknown): void
declare class AudioWorkletProcessor {
  readonly port: MessagePort
}

export type WorkletMessage =
  /** The wasm bytes: a compiled WebAssembly.Module does not reach a worklet in Chrome. */
  | { type: 'init'; bytes: ArrayBuffer }
  | { type: 'ops'; ops: WasmOp[] }

/** Render quanta between two meter messages (about 30 per second at 48 kHz). */
const METER_EVERY = 12

class FaulkProcessor extends AudioWorkletProcessor {
  private core: WasmCore | null = null
  private blocks = 0

  constructor() {
    super()
    this.port.onmessage = (event: MessageEvent<WorkletMessage>) => {
      const message = event.data
      if (message.type === 'init') {
        try {
          const instance = new WebAssembly.Instance(new WebAssembly.Module(message.bytes), {})
          this.core = new WasmCore(instance.exports as unknown as EngineExports, sampleRate)
          this.port.postMessage({ type: 'ready' })
        } catch (error) {
          // Errors in a worklet message handler are otherwise silent.
          this.port.postMessage({ type: 'error', message: String(error) })
        }
      } else if (this.core) {
        for (const op of message.ops) this.core.apply(op)
      }
    }
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const output = outputs[0]
    if (!this.core || !output || output.length === 0) return true
    this.core.render(output[0], output[1])
    // The only allocation on this thread: one small meter message every METER_EVERY quanta.
    if (++this.blocks % METER_EVERY === 0) {
      this.port.postMessage({ type: 'meter', peak: this.core.takePeak(), voices: this.core.activeVoices() })
    }
    return true
  }
}

registerProcessor('faulk-engine', FaulkProcessor)
