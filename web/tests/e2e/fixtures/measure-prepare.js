// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-nocheck: plain JavaScript loaded from a blob by engine-mo2.spec.ts only (MO-2, #55).
// The prepare side as src/lib/engine/prepare-worker.ts runs it, plus a resend of the rest
// of a stream the busy engine stopped (-9), so that a load larger than the rings goes through.
const wasi = (memory) => ({
  random_get(p, n) {
    const t = new Uint8Array(n)
    crypto.getRandomValues(t)
    new Uint8Array(memory.buffer, p, n).set(t)
    return 0
  },
  environ_get: () => 0,
  environ_sizes_get: () => 0,
  fd_write: () => 0,
  proc_exit: (c) => {
    throw new Error(`exit ${c}`)
  },
  sched_yield: () => 0,
})

let e = null
let memory = null
let ch = 0
const queue = []
const codes = []

function pump() {
  while (queue.length > 0) {
    const bytes = queue[0]
    const at = e.fk_inbox(ch, bytes.length)
    new Uint8Array(memory.buffer, at, bytes.length).set(bytes)
    const code = e.fk_push(ch, at, bytes.length)
    if (code === -9) {
      queue[0] = bytes.slice(e.fk_stopped_at(ch))
      return
    }
    if (code !== 0) codes.push(code)
    queue.shift()
  }
}

onmessage = (event) => {
  const m = event.data
  try {
    if (m.type === 'init') {
      memory = m.memory
      e = new WebAssembly.Instance(new WebAssembly.Module(m.bytes), { env: { memory }, wasi_snapshot_preview1: wasi(memory) }).exports
      e.fk_set_role(1)
      ch = e.fk_channel_new(m.sampleRate)
      const stack = e.fk_alloc(1 << 20, 16)
      const tls = e.fk_alloc(e.__tls_size.value, e.__tls_align.value)
      setInterval(() => {
        e.fk_flush(ch)
        pump()
      }, 5)
      postMessage({ type: 'ready', setup: { channel: ch, stackTop: stack + (1 << 20), tls } })
    } else if (m.type === 'push') {
      for (const bytes of m.streams) queue.push(new Uint8Array(bytes))
      pump()
      postMessage({ type: 'pushed' })
    } else if (m.type === 'counters') {
      postMessage({
        type: 'counters',
        queued: queue.length,
        codes,
        rejected: e.fk_prepare_counter(ch, 0),
        pending: e.fk_prepare_counter(ch, 1),
        prepareAllocs: e.fk_allocs(1),
        prepareFrees: e.fk_frees(1),
      })
    }
  } catch (error) {
    postMessage({ type: 'error', message: String(error) })
  }
}
