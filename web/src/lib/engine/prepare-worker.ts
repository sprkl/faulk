// The prepare side of the web engine (MO-2), in a dedicated Worker: the first instance of the
// engine module on the shared memory. It creates the channel, gives the worklet's instance
// its stack and thread-local block, and submits the ControlMsg byte streams the page sends.
// It allocates freely: it is off the audio path.
import { DiskExecutor, type Dir } from '../audio/stream/executor'
import { createChannel, instantiate } from './engine-abi'
import { PushQueue } from './push-queue'

export type ToPrepare =
  | { type: 'init'; bytes: ArrayBuffer; memory: WebAssembly.Memory; sampleRate: number }
  /** A stream of encoded ControlMsg (the page's command layer made it, never this Worker). */
  | { type: 'push'; bytes: ArrayBuffer }
  /** The project's folder, where the disk side reads the audio files (AU-3); `null`: none. */
  | { type: 'folder'; handle: FileSystemDirectoryHandle | null }

export type FromPrepare =
  | { type: 'ready'; channel: number; stackTop: number; tls: number; feedback: number; feedbackLen: number }
  /** A stream stopped by `code` (`MsgError::code`) at byte `at`: a busy engine is resent, so -9 here is a stream refused past the queue's bound (`PushQueue`). */
  | { type: 'pushed'; code: number; at: number }
  | { type: 'error'; message: string }

/** How often the prepare side pushes what waited and drops the garbage, in ms. */
export const FLUSH_MS = 5
/** How often the disk side plans and starts its reads, in ms (AU-3). */
export const DISK_MS = 10

declare const self: {
  onmessage: ((event: MessageEvent<ToPrepare>) => void) | null
  postMessage(message: FromPrepare): void
}

let queue: PushQueue | null = null
let disk: DiskExecutor | null = null
/** A folder given before the engine was ready. */
let folder: Dir | null = null

self.onmessage = (event) => {
  const message = event.data
  try {
    if (message.type === 'init') {
      const exports = instantiate(new WebAssembly.Module(message.bytes), message.memory)
      const setup = createChannel(exports, message.sampleRate)
      const channel = setup.channel
      queue = new PushQueue(exports, message.memory, channel, (code, at) => self.postMessage({ type: 'pushed', code, at }))
      setInterval(() => queue?.flush(), FLUSH_MS)
      disk = new DiskExecutor(exports, message.memory, channel)
      disk.folder(folder)
      setInterval(() => disk?.tick(), DISK_MS)
      self.postMessage({
        type: 'ready',
        ...setup,
        feedback: exports.fk_feedback(channel),
        feedbackLen: exports.fk_feedback_len(channel),
      })
    } else if (message.type === 'push' && queue) {
      queue.push(new Uint8Array(message.bytes))
    } else if (message.type === 'folder') {
      folder = message.handle as unknown as Dir | null
      disk?.folder(folder)
    }
  } catch (error) {
    self.postMessage({ type: 'error', message: String(error) })
  }
}
