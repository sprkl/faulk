// Runs on the audio thread, inside an AudioWorkletGlobalScope: the render instance of the
// engine module (MO-2). It knows no command: the prepare Worker feeds the engine through
// the shared memory, the page reads the engine's report from it. Once initialized (the
// engine's message path warmed up on a private channel, #355), nothing
// here posts a message or allocates, except re-taking the two output views after the memory
// grew (one comparison per block, CT-3). Each block's render is timed with the scope's clock
// for the engine's CPU load (#486): two clock reads and plain numbers per block.
import {
  MAX_FRAMES,
  becomeRenderInstance,
  blockSeconds,
  instantiate,
  renderClock,
  warmUp,
  type EngineExports,
  type RenderClock,
  type RenderSetup,
} from './engine-abi'

declare function registerProcessor(name: string, processor: unknown): void
/** The sample rate of the audio context, a global of the AudioWorkletGlobalScope. */
declare const sampleRate: number
/** The context frame of the block being rendered, a global of the AudioWorkletGlobalScope. */
declare const currentFrame: number
declare class AudioWorkletProcessor {
  readonly port: MessagePort
}

export type WorkletMessage = {
  type: 'init'
  /** The module's bytes: a compiled WebAssembly.Module does not reach a worklet in Chrome. */
  bytes: ArrayBuffer
  memory: WebAssembly.Memory
  setup: RenderSetup
}

class FaulkProcessor extends AudioWorkletProcessor {
  private exports: EngineExports | null = null
  private memory: WebAssembly.Memory | null = null
  private channel = 0
  private buffer: ArrayBufferLike | null = null
  private left = new Float32Array(0)
  private right = new Float32Array(0)
  private clock: RenderClock | null = null

  constructor() {
    super()
    // The one-time instantiation, before any audio (the only message this worklet handles).
    this.port.onmessage = (event: MessageEvent<WorkletMessage>) => {
      const message = event.data
      try {
        const exports = instantiate(new WebAssembly.Module(message.bytes), message.memory)
        becomeRenderInstance(exports, message.setup)
        // V8 compiles a wasm function at its first call: run every message once on a private
        // channel now, so that the first block that applies a message does not pay for it on
        // the audio thread (#355). It touches nothing of the real channel and sounds nowhere.
        // A failure (even a trap) costs only that first block's time, so it does not stop the engine.
        const warm = warmUp(exports, sampleRate)
        // The clock that times each block (#486), its step measured now, before any audio.
        const clock = renderClock()
        exports.fk_set_clock_resolution(message.setup.channel, clock.step)
        this.clock = clock
        this.memory = message.memory
        this.channel = message.setup.channel
        this.exports = exports
        this.takeViews()
        this.port.postMessage(warm === 0 ? { type: 'ready' } : { type: 'ready', warm })
      } catch (error) {
        // Errors in a worklet message handler are otherwise silent.
        this.port.postMessage({ type: 'error', message: String(error) })
      }
    }
  }

  /** Re-takes the output views when the shared memory grew (a new `buffer` object). */
  private takeViews() {
    const buffer = this.memory!.buffer
    if (buffer === this.buffer) return
    this.buffer = buffer
    this.left = new Float32Array(buffer, this.exports!.fk_left(this.channel), MAX_FRAMES)
    this.right = new Float32Array(buffer, this.exports!.fk_right(this.channel), MAX_FRAMES)
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const output = outputs[0]
    const clock = this.clock
    if (!this.exports || !clock || !output || output.length === 0) return true
    const outLeft = output[0]
    const outRight = output[1]
    // The engine learns the context frame of this block, the clock the page stamps live notes in (#499).
    const start = clock.now()
    const frames = this.exports.fk_render_at(this.channel, outLeft.length, currentFrame)
    this.exports.fk_add_render_time(this.channel, blockSeconds(start, clock.now()), frames)
    this.takeViews()
    const left = this.left
    const right = this.right
    for (let i = 0; i < frames; i++) {
      outLeft[i] = left[i]
      if (outRight) outRight[i] = right[i]
    }
    return true
  }
}

registerProcessor('faulk-engine', FaulkProcessor)
