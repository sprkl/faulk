// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-nocheck: a plain-JavaScript prototype loaded by the e2e suite only (CT-3, #39), not part of the app.
/* global AudioWorkletProcessor, registerProcessor, sampleRate */
// CT-3 prototype worklet: the render side of the two ways of § 2.3, measured with the
// shared clock of clock-worker.js (ticks, calibrated by the page into microseconds).
//
// Way (a), plain memory: `onmessage` copies the transferred bytes into a preallocated inbox
// and applies them; a `grow` message runs `prepare` here, between two quanta.
// Way (b), shared memory: this is the second instance of the module on the Worker's
// memory, with its own stack; `process` starts by pumping the rt ring.
//
// `process` never allocates: the views are re-taken only when `memory.buffer` changed
// (a growth), and the stats are plain numbers updated in place.

const wasi = () => ({
  environ_get: () => 0,
  environ_sizes_get: () => 0,
  fd_write: () => 0,
  proc_exit: (code) => {
    throw new Error(`wasm exit ${code}`)
  },
  sched_yield: () => 0,
})

const MAX_FRAMES = 2048
const INBOX_BYTES = 64 << 10

class FaulkProto extends AudioWorkletProcessor {
  constructor() {
    super()
    this.exports = null
    this.mode = null
    this.inst = 0
    this.proto = 0
    this.clock = null
    this.ticksPerUs = 1
    this.inbox = new Uint8Array(INBOX_BYTES)
    this.inboxView = new DataView(this.inbox.buffer)
    this.left = null
    this.right = null
    this.buffer = null
    this.resetStats()
    this.port.onmessage = (event) => this.onMessage(event.data)
  }

  resetStats() {
    this.stats = {
      blocks: 0,
      lateBlocks: 0,
      gapMax: 0,
      gapSum: 0,
      gapCount: 0,
      pumpTicks: 0,
      pumpMax: 0,
      pumpCalls: 0,
      pumped: 0,
      msgTicks: 0,
      msgCount: 0,
      bursts: {},
      grows: [],
      fills: [],
      viewsRefreshed: 0,
      peak: 0,
      silentBlocks: 0,
    }
    this.lastEntry = -1
  }

  now() {
    // A wrapping 32-bit counter read as a plain number: no BigInt, nothing allocated.
    return Atomics.load(this.clock, 0) >>> 0
  }

  takeViews() {
    const buffer = this.exports.memory.buffer
    if (buffer === this.buffer) return
    // A grown plain memory detaches the old buffer (byteLength 0); a grown shared memory
    // keeps the old one readable. Either way the views are re-taken from the new buffer.
    this.buffer = buffer
    this.left = new Float32Array(buffer, this.exports.fk_left_ptr(this.inst), MAX_FRAMES)
    this.right = new Float32Array(buffer, this.exports.fk_right_ptr(this.inst), MAX_FRAMES)
    this.stats.viewsRefreshed++
  }

  onMessage(message) {
    try {
      if (message.type === 'init') {
        this.mode = message.mode
        this.clock = new Int32Array(message.clock)
        this.ticksPerUs = message.ticksPerUs
        // Blocks later than this since the previous one count as late: 1.5 render quanta.
        this.lateTicks = Math.round((1.5 * 128 * 1e6 * this.ticksPerUs) / sampleRate)
        const module = new WebAssembly.Module(message.bytes)
        if (this.mode === 'a') {
          this.exports = new WebAssembly.Instance(module, {}).exports
          this.proto = this.exports.fk_proto_new(1)
          this.inst = this.exports.fk_new(sampleRate)
          // The role (and its allocation counter) starts after the one-time setup, so that
          // the counter holds what prepare-between-quanta costs, not the rings and instance.
          this.exports.fk_proto_set_role(2)
        } else {
          const instance = new WebAssembly.Instance(module, {
            env: { memory: message.memory },
            wasi_snapshot_preview1: wasi(),
          })
          this.exports = instance.exports
          this.exports.__stack_pointer.value = message.stackTop
          this.exports.fk_proto_set_role(2)
          this.proto = message.proto
          this.inst = message.inst
        }
        this.exports.fk_note_on(this.inst, 60, 100)
        this.takeViews()
        this.resetStats()
        this.port.postMessage({ type: 'ready', role: this.exports.fk_proto_role() })
      } else if (message.type === 'msg') {
        // Way (a): bounded bytes copied into the preallocated inbox, then applied.
        const t0 = this.now()
        const bytes = new Uint8Array(message.buf)
        const n = Math.min(bytes.length, INBOX_BYTES)
        this.inbox.set(bytes.subarray(0, n))
        const param = this.inboxView.getUint32(0, true)
        const value = this.inboxView.getFloat32(4, true)
        this.exports.fk_set_param(this.inst, param, value)
        const t1 = this.now()
        this.stats.msgTicks += (t1 - t0) >>> 0
        this.stats.msgCount++
        const burst = (this.stats.bursts[message.burst] ??= { first: t0, last: t1, n: 0 })
        burst.last = t1
        burst.n++
      } else if (message.type === 'grow') {
        // Way (a): prepare runs here, on the render thread, between two quanta.
        const before = this.exports.memory.buffer.byteLength
        const t0 = this.now()
        const notes = this.exports.fk_proto_prepare_notes(this.proto, message.notes)
        const tracks = this.exports.fk_proto_prepare_tracks(this.proto, message.tracks)
        const pumped = this.exports.fk_proto_pump(this.proto, this.inst)
        const collected = this.exports.fk_proto_collect(this.proto)
        const t1 = this.now()
        const detached = this.buffer.byteLength === 0
        this.stats.grows.push({
          us: ((t1 - t0) >>> 0) / this.ticksPerUs,
          before,
          after: this.exports.memory.buffer.byteLength,
          notes,
          tracks,
          pumped,
          collected,
          detached,
        })
      } else if (message.type === 'fill') {
        // Way (a): `n` boxed tracks prepared here, between two quanta, then applied.
        const t0 = this.now()
        let failed = 0
        for (let i = 0; i < message.n; i++) failed += this.exports.fk_proto_prepare_track(this.proto, i)
        let pumped = 0
        while (pumped < message.n) {
          const n = this.exports.fk_proto_pump(this.proto, this.inst)
          if (n === 0) break
          pumped += n
        }
        this.stats.fills.push({ us: ((this.now() - t0) >>> 0) / this.ticksPerUs, n: message.n, failed, pumped })
      } else if (message.type === 'report') {
        const s = this.stats
        this.port.postMessage({
          type: 'report',
          blocks: s.blocks,
          lateBlocks: s.lateBlocks,
          gapMaxUs: s.gapMax / this.ticksPerUs,
          gapMeanUs: s.gapCount ? s.gapSum / s.gapCount / this.ticksPerUs : 0,
          pumpCalls: s.pumpCalls,
          pumped: s.pumped,
          pumpUsPerCall: s.pumpCalls ? s.pumpTicks / s.pumpCalls / this.ticksPerUs : 0,
          pumpUsPerMsg: s.pumped ? s.pumpTicks / s.pumped / this.ticksPerUs : 0,
          pumpMaxUs: s.pumpMax / this.ticksPerUs,
          msgCount: s.msgCount,
          msgUsHandler: s.msgCount ? s.msgTicks / s.msgCount / this.ticksPerUs : 0,
          bursts: Object.values(s.bursts).map((b) => ({ n: b.n, usPerMsg: ((b.last - b.first) >>> 0) / this.ticksPerUs / b.n })),
          grows: s.grows,
          fills: s.fills,
          skipped: this.exports.fk_proto_skipped(this.proto),
          garbageRefused: this.exports.fk_proto_garbage_refused(this.proto),
          viewsRefreshed: s.viewsRefreshed,
          peak: s.peak,
          silentBlocks: s.silentBlocks,
          renderAllocs: this.exports.fk_proto_allocs(2),
          renderFrees: this.exports.fk_proto_frees(2),
          applied: this.exports.fk_proto_applied(this.proto),
          notes: this.exports.fk_proto_notes_len(this.proto),
          tracksCapacity: this.exports.fk_proto_tracks_capacity(this.proto),
          tracksLen: this.exports.fk_proto_tracks_len(this.proto),
          memoryBytes: this.exports.memory.buffer.byteLength,
          sampleRate,
        })
      } else if (message.type === 'reset') {
        this.resetStats()
        this.port.postMessage({ type: 'reset-done' })
      }
    } catch (error) {
      this.port.postMessage({ type: 'error', message: String(error) })
    }
  }

  process(_inputs, outputs) {
    if (!this.exports) return true
    const s = this.stats
    const entry = this.now()
    if (this.lastEntry >= 0) {
      const gap = (entry - this.lastEntry) >>> 0
      if (gap > s.gapMax) s.gapMax = gap
      s.gapSum += gap
      s.gapCount++
      if (gap > this.lateTicks) s.lateBlocks++
    }
    this.lastEntry = entry
    if (this.mode === 'b') {
      const t0 = this.now()
      const n = this.exports.fk_proto_pump(this.proto, this.inst)
      const t1 = this.now()
      const took = (t1 - t0) >>> 0
      s.pumpTicks += took
      if (took > s.pumpMax) s.pumpMax = took
      s.pumpCalls++
      s.pumped += n
    }
    const output = outputs[0]
    const frames = output[0].length
    this.exports.fk_render(this.inst, frames)
    this.takeViews()
    let peak = 0
    const left = this.left
    const right = this.right
    const outL = output[0]
    const outR = output[1]
    for (let i = 0; i < frames; i++) {
      const l = left[i]
      outL[i] = l
      if (outR) outR[i] = right[i]
      const a = l < 0 ? -l : l
      if (a > peak) peak = a
    }
    if (peak > s.peak) s.peak = peak
    if (peak === 0) s.silentBlocks++
    s.blocks++
    return true
  }
}

registerProcessor('faulk-proto', FaulkProto)
