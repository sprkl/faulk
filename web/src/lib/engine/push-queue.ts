// The prepare Worker's queue of ControlMsg streams (MO-2). A stream the engine stops because
// it is busy (`EngineBusy`, -9: its ring and its own queue are full, as a big project's snapshot
// makes them) keeps its rest, from the message that stopped it (`fk_stopped_at`, the refused
// message left nothing behind), and is sent again at the next flush, before anything that came
// after it: nothing is lost nor reordered (#245). Any other stop is reported and the stream
// dropped (PR-3 / PR-5 resync). The native hosts do the same with `engine::host::StreamQueue`.
import type { EngineExports } from './engine-abi'

/** `MsgError::EngineBusy`. */
export const ENGINE_BUSY = -9
/** `MsgError::OutOfMemory`. */
export const OUT_OF_MEMORY = -8
/**
 * The most bytes of streams kept waiting behind the one the engine is taking (#245, `MAX_WAITING_BYTES`
 * of `engine::host`): a stream that would go past it is refused and reported as a busy engine, so that an
 * engine that does not consume cannot make the queue grow without limit. The stream the engine is taking is
 * never refused, whatever its size: a big project's snapshot may be larger.
 */
export const MAX_WAITING_BYTES = 64 * 1024 * 1024

/** `msg::MSG_VERSION` and the header's tag of `Reset` (`crates/engine/src/msg.rs`). */
const MSG_VERSION = 1
const HEADER_BYTES = 16

/** Whether a stream's first message is a `Reset`. */
export function startsWithReset(bytes: Uint8Array): boolean {
  return bytes.length >= HEADER_BYTES && bytes[0] === MSG_VERSION && bytes[2] === 0 && bytes[3] === 0
}

type Exports = Pick<EngineExports, 'fk_inbox' | 'fk_push' | 'fk_stopped_at' | 'fk_flush'>

export class PushQueue {
  private readonly queue: Uint8Array[] = []
  private bytes = 0

  constructor(
    private readonly exports: Exports,
    private readonly memory: { buffer: ArrayBufferLike },
    private readonly channel: number,
    /** A stream stopped for another reason than a busy engine, or refused past the bound: its code and byte offset. */
    private readonly onStopped: (code: number, at: number) => void,
    private readonly maxWaiting = MAX_WAITING_BYTES,
  ) {}

  /** Streams waiting for the engine. */
  get waiting(): number {
    return this.queue.length
  }

  /** Bytes of the waiting streams not sent yet. */
  get waitingBytes(): number {
    return this.bytes
  }

  push(bytes: Uint8Array) {
    if (bytes.length === 0) return
    // A stream that starts the engine over (a `Reset` and a snapshot: an open, a switch, a restart) makes what
    // waits pointless: the `Reset` would wipe it once applied.
    if (startsWithReset(bytes)) {
      this.queue.length = 0
      this.bytes = 0
    }
    if (this.queue.length > 0 && this.bytes - this.queue[0].length + bytes.length > this.maxWaiting) {
      // Nothing of it is handled: the engine is behind the project, the page restarts it from a snapshot.
      this.onStopped(ENGINE_BUSY, 0)
      return
    }
    this.queue.push(bytes)
    this.bytes += bytes.length
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
        this.shift()
        this.onStopped(OUT_OF_MEMORY, 0)
        continue
      }
      new Uint8Array(this.memory.buffer, inbox, bytes.length).set(bytes)
      const code = this.exports.fk_push(this.channel, inbox, bytes.length)
      if (code === ENGINE_BUSY) {
        const at = Math.min(Math.max(0, this.exports.fk_stopped_at(this.channel)), bytes.length)
        this.queue[0] = bytes.subarray(at)
        this.bytes -= at
        return
      }
      this.shift()
      if (code !== 0) this.onStopped(code, this.exports.fk_stopped_at(this.channel))
    }
  }

  private shift() {
    this.bytes -= this.queue.shift()?.length ?? 0
  }
}
