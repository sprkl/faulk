import type { Command } from '../generated/protocol'
import type { Engine, EngineEvents, EngineStatus } from './engine'
import { engineMemory } from './engine-abi'
import { readFeedback } from './feedback'
import type { FromPrepare, ToPrepare } from './prepare-worker'
import type { WorkletMessage } from './worklet'
import workletUrl from './worklet.ts?worker&url'
import wasmUrl from './wasm/engine.wasm?url'

type FromWorklet = { type: 'ready' } | { type: 'error'; message: string }

/** How often the page reads the engine's report, in ms (the engine writes ~30 per second). */
const FEEDBACK_MS = 33

/**
 * The engine compiled to WebAssembly (MO-2): one module, two instances on one shared memory.
 * A Worker holds the prepare side and takes the ControlMsg streams the page's project core
 * encodes in Rust (`project/core.ts`, owned by the dispatcher); the AudioWorklet holds the render side; the page reads the report
 * from the shared memory. Nothing crosses the audio thread by `postMessage` once it runs.
 */
export class WebEngine implements Engine {
  readonly kind = 'web' as const
  private readonly timer: ReturnType<typeof setInterval>

  private constructor(
    private readonly context: AudioContext,
    private readonly worker: Worker,
    memory: WebAssembly.Memory,
    feedback: { at: number; len: number },
    events: EngineEvents,
  ) {
    let last = 0
    this.timer = setInterval(() => {
      const report = readFeedback(memory.buffer, feedback.at, feedback.len)
      if (!report || report.seq === last) return
      last = report.seq
      events.onMeter(Math.max(0, ...report.peaks), report.voices)
    }, FEEDBACK_MS)
  }

  /** Must be called from a user gesture (browsers start audio only then). */
  static async create(events: EngineEvents): Promise<WebEngine> {
    const context = new AudioContext({ latencyHint: 'interactive' })
    let worker: Worker | null = null
    try {
      return await WebEngine.start(context, events, (w) => (worker = w))
    } catch (error) {
      // Nothing left behind: the Worker's flush timer and the audio context.
      ;(worker as Worker | null)?.terminate()
      await context.close().catch(() => {})
      throw error
    }
  }

  private static async start(context: AudioContext, events: EngineEvents, track: (w: Worker) => void): Promise<WebEngine> {
    const memory = engineMemory()
    const [engineWasm] = await Promise.all([
      fetch(wasmUrl).then((r) => r.arrayBuffer()),
      context.audioWorklet.addModule(workletUrl),
    ])
    const worker = new Worker(new URL('./prepare-worker.ts', import.meta.url), { type: 'module' })
    track(worker)
    const ready = await new Promise<Extract<FromPrepare, { type: 'ready' }>>((resolve, reject) => {
      worker.onmessage = (event: MessageEvent<FromPrepare>) => {
        const data = event.data
        if (data.type === 'ready') resolve(data)
        else if (data.type === 'error') reject(new Error(`engine worker: ${data.message}`))
      }
      worker.onerror = (event) => reject(new Error(`engine worker: ${event.message}`))
      worker.postMessage({ type: 'init', bytes: engineWasm.slice(0), memory, sampleRate: context.sampleRate } satisfies ToPrepare)
    })
    worker.onmessage = (event: MessageEvent<FromPrepare>) => {
      const data = event.data
      // A stream the engine stopped (`MsgError::code`): out of memory, engine busy, corrupt.
      // Surfaced to the dispatcher, which stops this engine (restart = a new engine + snapshot).
      if (data.type === 'pushed') {
        console.error(`engine refused a stream: code ${data.code} at byte ${data.at}`)
        events.onStreamRefused?.(data.code, data.at)
      } else if (data.type === 'error') console.error(`engine worker: ${data.message}`)
    }

    const node = new AudioWorkletNode(context, 'faulk-engine', {
      numberOfInputs: 0,
      outputChannelCount: [2],
    })
    await new Promise<void>((resolve, reject) => {
      node.port.onmessage = (event: MessageEvent<FromWorklet>) => {
        const data = event.data
        if (data.type === 'ready') resolve()
        else reject(new Error(`audio worklet: ${data.message}`))
      }
      const setup = { channel: ready.channel, stackTop: ready.stackTop, tls: ready.tls }
      node.port.postMessage({ type: 'init', bytes: engineWasm, memory, setup } satisfies WorkletMessage, [engineWasm])
    })
    node.port.onmessage = null

    const engine = new WebEngine(context, worker, memory, { at: ready.feedback, len: ready.feedbackLen }, events)
    node.connect(context.destination)
    await context.resume()
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

  /** Takes the engine bytes the project core encoded for a command (the command itself is not needed). */
  send(_command: Command, bytes: Uint8Array) {
    this.push(bytes)
  }

  /** Pushes a stream of engine messages: a command's, or a snapshot to start from or to catch up. */
  push(bytes: Uint8Array) {
    if (bytes.length === 0) return
    const buffer = bytes.buffer as ArrayBuffer
    this.worker.postMessage({ type: 'push', bytes: buffer } satisfies ToPrepare, [buffer])
  }

  /** Pauses audio without losing the engine (used while the helper plays). */
  async suspend() {
    await this.context.suspend()
  }

  async resume() {
    await this.context.resume()
  }

  async close() {
    clearInterval(this.timer)
    await this.context.close()
    this.worker.terminate()
  }
}
