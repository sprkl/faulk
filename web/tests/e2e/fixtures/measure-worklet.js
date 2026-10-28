// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-nocheck: plain JavaScript loaded from a blob by engine-mo2.spec.ts and tests/bench/engine-swap.bench.ts (MO-2, #55; QA-7, #130).
/* global AudioWorkletProcessor, registerProcessor, sampleRate */
// What src/lib/engine/worklet.ts does, with the pump timed apart: `fk_render` is exactly
// `fk_pump` then `fk_render_block`, and the clock (a Worker spinning on a shared counter, as
// in CT-3: an AudioWorkletGlobalScope has no `performance`) is read around `fk_pump`.
// `process` allocates nothing: plain numbers updated in place.
// The counter's rate is not constant: the spinning Worker runs about six times slower while its
// loop is still in V8's baseline tier (a few hundred ms after it starts), then speeds up and
// follows the core's frequency (#283). The page therefore calibrates over the very window it
// measures and hands the rate over (`rate`) just before it asks for the report.
// The first pump that applies anything after a reset is kept apart (`firstUs`): V8 compiles a
// wasm function on its first call (lazy compilation), so that pump runs code for the first time
// and costs several times a warm one; `pumpMaxUs` is the longest of the others.
const wasi = () => ({
  random_get: () => 0,
  environ_get: () => 0,
  environ_sizes_get: () => 0,
  fd_write: () => 0,
  proc_exit: (c) => {
    throw new Error(`exit ${c}`)
  },
  sched_yield: () => 0,
})
const MAX_FRAMES = 2048

class Measure extends AudioWorkletProcessor {
  constructor() {
    super()
    this.e = null
    this.stats = { blocks: 0, silent: 0, pumpTicks: 0, busyTicks: 0, pumpMax: 0, firstTicks: 0, firstApplied: 0, pumpCalls: 0, applied: 0, maxApplied: 0, views: 0, peak: 0 }
    this.port.onmessage = (event) => {
      const m = event.data
      try {
        if (m.type === 'init') {
          const e = new WebAssembly.Instance(new WebAssembly.Module(m.bytes), { env: { memory: m.memory }, wasi_snapshot_preview1: wasi() }).exports
          e.__stack_pointer.value = m.setup.stackTop
          e.__wasm_init_tls(m.setup.tls)
          e.fk_set_role(2)
          // The warm-up of the apply path (#355), as src/lib/engine/worklet.ts runs it; `warm: false`
          // measures the cold start it removes.
          this.warmCode = m.warm === false ? null : e.fk_warm(sampleRate)
          this.memory = m.memory
          this.ch = m.setup.channel
          this.clock = new Int32Array(m.clock)
          this.ticksPerUs = m.ticksPerUs
          this.buffer = null
          this.e = e
          this.allocs0 = e.fk_allocs(2)
          this.frees0 = e.fk_frees(2)
          this.port.postMessage({ type: 'ready', role: e.fk_role(), warm: this.warmCode })
        } else if (m.type === 'rate') {
          this.ticksPerUs = m.ticksPerUs
          this.port.postMessage({ type: 'rate' })
        } else if (m.type === 'reset') {
          for (const k of Object.keys(this.stats)) this.stats[k] = 0
          this.port.postMessage({ type: 'reset' })
        } else if (m.type === 'report') {
          const s = this.stats
          const e = this.e
          this.port.postMessage({
            type: 'report',
            ...s,
            pumpUsPerMsg: s.applied ? s.busyTicks / s.applied / this.ticksPerUs : 0,
            pumpUsPerCall: s.pumpCalls ? s.pumpTicks / s.pumpCalls / this.ticksPerUs : 0,
            pumpMaxUs: s.pumpMax / this.ticksPerUs,
            firstUs: s.firstTicks / this.ticksPerUs,
            renderAllocs: e.fk_allocs(2) - this.allocs0,
            renderFrees: e.fk_frees(2) - this.frees0,
            hostApplied: e.fk_host_counter(this.ch, 0),
            skipped: e.fk_host_counter(this.ch, 1),
            tracksCapacity: e.fk_host_counter(this.ch, 3),
            tracksHeld: e.fk_host_counter(this.ch, 4),
          })
        }
      } catch (error) {
        this.port.postMessage({ type: 'error', message: String(error) })
      }
    }
  }

  now() {
    return Atomics.load(this.clock, 0) >>> 0
  }

  process(_inputs, outputs) {
    const e = this.e
    if (!e) return true
    const s = this.stats
    const t0 = this.now()
    const applied = e.fk_pump(this.ch)
    const took = (this.now() - t0) >>> 0
    s.pumpTicks += took
    s.pumpCalls++
    if (applied > 0) {
      s.busyTicks += took
      s.applied += applied
      if (s.firstApplied === 0) {
        s.firstTicks = took
        s.firstApplied = applied
      } else if (took > s.pumpMax) s.pumpMax = took
      if (applied > s.maxApplied) s.maxApplied = applied
    }
    const out = outputs[0]
    const frames = e.fk_render_block(this.ch, out[0].length)
    if (this.memory.buffer !== this.buffer) {
      this.buffer = this.memory.buffer
      this.left = new Float32Array(this.buffer, e.fk_left(this.ch), MAX_FRAMES)
      this.right = new Float32Array(this.buffer, e.fk_right(this.ch), MAX_FRAMES)
      s.views++
    }
    let peak = 0
    for (let i = 0; i < frames; i++) {
      const l = this.left[i]
      out[0][i] = l
      if (out[1]) out[1][i] = this.right[i]
      const a = l < 0 ? -l : l
      if (a > peak) peak = a
    }
    if (peak > s.peak) s.peak = peak
    if (peak === 0) s.silent++
    s.blocks++
    return true
  }
}

registerProcessor('faulk-measure', Measure)
