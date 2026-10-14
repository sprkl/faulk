// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-nocheck: a plain-JavaScript prototype loaded by the e2e suite only (CT-3, #39), not part of the app.
// Way (b) of the CT-3 prototype: the prepare side. This worker holds the first instance of
// the shared-memory module (built for wasm32-wasip1-threads on stable); it creates the rings,
// the engine instance and a stack for the worklet's instance, then pushes messages and
// builds the tables that `prepare` would, while the worklet only pops and swaps.
const wasi = () => ({
  environ_get: () => 0,
  environ_sizes_get: () => 0,
  fd_write: () => 0,
  proc_exit: (code) => {
    throw new Error(`wasm exit ${code}`)
  },
  sched_yield: () => 0,
})

let exports = null
let memory = null
let proto = 0
let inst = 0

onmessage = (event) => {
  const message = event.data
  try {
    if (message.type === 'init') {
      memory = message.memory
      const module = new WebAssembly.Module(message.bytes)
      const instance = new WebAssembly.Instance(module, { env: { memory }, wasi_snapshot_preview1: wasi() })
      exports = instance.exports
      exports.fk_proto_set_role(1)
      proto = exports.fk_proto_new(1)
      inst = exports.fk_new(message.sampleRate)
      // The worklet's instance needs its own stack: 1 MiB from the shared heap, top address.
      const stackBytes = 1 << 20
      const stackTop = exports.fk_proto_alloc(stackBytes) + stackBytes
      postMessage({ type: 'ready', proto, inst, stackTop, exports: Object.keys(exports) })
    } else if (message.type === 'burst') {
      const t0 = performance.now()
      let busy = 0
      for (let i = 0; i < message.n; i++) busy += exports.fk_proto_push_param(proto, message.param, i / message.n)
      postMessage({ type: 'burst-done', ms: performance.now() - t0, busy })
    } else if (message.type === 'grow') {
      const before = memory.buffer.byteLength
      const t0 = performance.now()
      const notes = exports.fk_proto_prepare_notes(proto, message.notes)
      const tracks = exports.fk_proto_prepare_tracks(proto, message.tracks)
      const ms = performance.now() - t0
      postMessage({ type: 'grow-done', ms, before, after: memory.buffer.byteLength, notes, tracks })
    } else if (message.type === 'track') {
      postMessage({ type: 'track-done', result: exports.fk_proto_prepare_track(proto, message.slot) })
    } else if (message.type === 'collect') {
      postMessage({ type: 'collect-done', collected: exports.fk_proto_collect(proto) })
    } else if (message.type === 'counters') {
      postMessage({
        type: 'counters-done',
        prepare: { allocs: exports.fk_proto_allocs(1), frees: exports.fk_proto_frees(1) },
        render: { allocs: exports.fk_proto_allocs(2), frees: exports.fk_proto_frees(2) },
        busy: exports.fk_proto_busy(proto),
        applied: exports.fk_proto_applied(proto),
        notes: exports.fk_proto_notes_len(proto),
        tracksLen: exports.fk_proto_tracks_len(proto),
        tracksCapacity: exports.fk_proto_tracks_capacity(proto),
        role: exports.fk_proto_role(),
      })
    }
  } catch (error) {
    postMessage({ type: 'error', message: String(error) })
  }
}
