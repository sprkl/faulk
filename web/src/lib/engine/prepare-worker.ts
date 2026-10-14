// The prepare side of the web engine (MO-2), in a dedicated Worker: the first instance of the
// engine module on the shared memory. It creates the channel, gives the worklet's instance
// its stack and thread-local block, and submits the ControlMsg byte streams the page sends.
// It allocates freely: it is off the audio path.
import { createChannel, instantiate, type EngineExports } from './engine-abi'

export type ToPrepare =
  | { type: 'init'; bytes: ArrayBuffer; memory: WebAssembly.Memory; sampleRate: number }
  /** A stream of encoded ControlMsg (the page's command layer made it, never this Worker). */
  | { type: 'push'; bytes: ArrayBuffer }

export type FromPrepare =
  | { type: 'ready'; channel: number; stackTop: number; tls: number; feedback: number; feedbackLen: number }
  /** A push stopped by `code` (`MsgError::code`) at byte `at` of its stream. */
  | { type: 'pushed'; code: number; at: number }
  | { type: 'error'; message: string }

/** How often the prepare side pushes what waited and drops the garbage, in ms. */
export const FLUSH_MS = 5

declare const self: {
  onmessage: ((event: MessageEvent<ToPrepare>) => void) | null
  postMessage(message: FromPrepare): void
}

let exports: EngineExports | null = null
let memory: WebAssembly.Memory | null = null
let channel = 0

self.onmessage = (event) => {
  const message = event.data
  try {
    if (message.type === 'init') {
      memory = message.memory
      exports = instantiate(new WebAssembly.Module(message.bytes), memory)
      const setup = createChannel(exports, message.sampleRate)
      channel = setup.channel
      setInterval(() => exports?.fk_flush(channel), FLUSH_MS)
      self.postMessage({
        type: 'ready',
        ...setup,
        feedback: exports.fk_feedback(channel),
        feedbackLen: exports.fk_feedback_len(channel),
      })
    } else if (message.type === 'push' && exports && memory) {
      const bytes = new Uint8Array(message.bytes)
      const inbox = exports.fk_inbox(channel, bytes.length)
      if (!inbox) {
        self.postMessage({ type: 'pushed', code: -8, at: 0 })
        return
      }
      new Uint8Array(memory.buffer, inbox, bytes.length).set(bytes)
      const code = exports.fk_push(channel, inbox, bytes.length)
      if (code !== 0) self.postMessage({ type: 'pushed', code, at: exports.fk_stopped_at(channel) })
    }
  } catch (error) {
    self.postMessage({ type: 'error', message: String(error) })
  }
}
