// The prepare Worker's queue of ControlMsg streams (MO-2). A stream the engine stops because
// it is busy (`EngineBusy`, -9: its ring and its own queue are full) keeps its rest, from the
// message that stopped it (`fk_stopped_at`, the refused message left nothing behind), and is
// sent again at the next flush, before anything that came after it: nothing is lost nor
// reordered. Any other stop is reported and the stream dropped (PR-3 / PR-5 resync).
import type { EngineExports } from './engine-abi'

/** `MsgError::EngineBusy`. */
export const ENGINE_BUSY = -9
/** `MsgError::OutOfMemory`. */
export const OUT_OF_MEMORY = -8

type Exports = Pick<EngineExports, 'fk_inbox' | 'fk_push' | 'fk_stopped_at' | 'fk_flush'>

export class PushQueue {
  private readonly queue: Uint8Array[] = []

  constructor(
    private readonly exports: Exports,
    private readonly memory: { buffer: ArrayBufferLike },
    private readonly channel: number,
    /** A stream stopped for another reason than a busy engine: its code and byte offset. */
    private readonly onStopped: (code: number, at: number) => void,
  ) {}

  /** Streams waiting for the engine. */
  get waiting(): number {
    return this.queue.length
  }

  push(bytes: Uint8Array) {
    if (bytes.length > 0) this.queue.push(bytes)
    this.drain()
  }

  /** The Worker's regular tick: the engine's flush, then what waited here. */
  flush() {
    this.exports.fk_flush(this.channel)
    this.drain()
  }

  private drain() {
    while (this.queue.length > 0) {
      const bytes = this.queue[0]
      const inbox = this.exports.fk_inbox(this.channel, bytes.length)
      if (!inbox) {
        this.queue.shift()
        this.onStopped(OUT_OF_MEMORY, 0)
        continue
      }
      new Uint8Array(this.memory.buffer, inbox, bytes.length).set(bytes)
      const code = this.exports.fk_push(this.channel, inbox, bytes.length)
      if (code === ENGINE_BUSY) {
        const at = Math.max(0, this.exports.fk_stopped_at(this.channel))
        this.queue[0] = bytes.slice(at)
        return
      }
      this.queue.shift()
      if (code !== 0) this.onStopped(code, this.exports.fk_stopped_at(this.channel))
    }
  }
}
