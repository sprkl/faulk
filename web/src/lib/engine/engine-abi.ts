// The C ABI of crates/engine-wasm (MO-2): one module, two instances on one shared memory,
// the prepare instance in a Worker and the render instance in the AudioWorklet. Imported by
// the page, the Worker and the worklet: nothing here allocates once the instance exists.

/** Exports of the engine module (see crates/engine-wasm/src/lib.rs). */
export interface EngineExports {
  memory?: WebAssembly.Memory
  __stack_pointer: WebAssembly.Global
  __tls_size: WebAssembly.Global
  __tls_align: WebAssembly.Global
  __wasm_init_tls(block: number): void
  fk_set_role(role: number): void
  fk_role(): number
  fk_allocs(role: number): number
  fk_frees(role: number): number
  fk_alloc(bytes: number, align: number): number
  fk_channel_new(sampleRate: number): number
  fk_inbox(channel: number, len: number): number
  fk_push(channel: number, ptr: number, len: number): number
  fk_stopped_at(channel: number): number
  fk_flush(channel: number): void
  fk_prepare_counter(channel: number, which: number): number
  fk_pump(channel: number): number
  fk_render_block(channel: number, frames: number): number
  fk_render(channel: number, frames: number): number
  /** `fk_render` after telling the engine the host frame of the block's first sample (#499): the worklet's `currentFrame`. */
  fk_render_at(channel: number, frames: number, hostFrame: number): number
  /** One timed block (#486): the render of `frames` took `seconds` by the worklet's clock; the report's CPU load and peak follow. */
  fk_add_render_time(channel: number, seconds: number, frames: number): void
  /** The step of the worklet's clock in seconds (#486): it sets the length of the peak windows. */
  fk_set_clock_resolution(channel: number, seconds: number): void
  /** Runs every message once on a throwaway channel (#355): 0, or an error code. */
  fk_warm(sampleRate: number): number
  fk_left(channel: number): number
  fk_right(channel: number): number
  fk_feedback(channel: number): number
  fk_feedback_len(channel: number): number
  fk_host_counter(channel: number, which: number): number
  /** The disk planner's poll (AU-3, prepare instance): how many jobs it wrote into `fk_disk_jobs`. */
  fk_disk_poll(channel: number, nowMs: number): number
  fk_disk_jobs(channel: number): number
  fk_disk_inbox(channel: number, len: number): number
  fk_disk_done(channel: number, id: number, len: number, fileLen: number): number
  fk_disk_failed(channel: number, id: number, code: number): void
  fk_disk_set_budget(channel: number, bytes: number): void
}

/**
 * The offline export's exports (`fk_export_*`, issue #297): an engine of its own in the
 * instance of the export Worker, never in the worklet. Codes below 0 are `MsgError::code` or
 * -100 (a message rejected), -101 (the engine stuck), -102 (a length it refuses).
 */
export interface ExportExports extends EngineExports {
  fk_export_new(sampleRate: number, bits: number, bodyLo: number, bodyHi: number): number
  fk_export_free(ex: number): void
  fk_export_inbox(ex: number, len: number): number
  fk_export_submit(ex: number, len: number): number
  fk_export_set_stop(ex: number, len: number): number
  /** Renders up to `maxFrames`: the phase after (0 content, 1 tail, 2 done) or an error code. */
  fk_export_step(ex: number, maxFrames: number): number
  fk_export_pcm(ex: number): number
  fk_export_pcm_len(ex: number): number
  fk_export_header(ex: number): number
  fk_export_header_len(ex: number): number
  /**
   * 0 rendered, 1 written, 2 content frames, 3 peak (linear), 4 clipped, 5 tail cut,
   * 6 header bytes, 7 the output protection's peak gain reduction (linear, 0 = never engaged; #402),
   * 8 the count of frames the protection silenced for a non-finite sample (0 on a clean render; #402).
   */
  fk_export_stat(ex: number, which: number): number
}

/** Roles of the two instances, for the module's allocation counters. */
export const ROLE_PREPARE = 1
export const ROLE_RENDER = 2

/** Largest render quantum the engine accepts (`MAX_FRAMES`). */
export const MAX_FRAMES = 2048

/** Stack of the render instance, taken from the shared heap by the prepare instance. */
export const RENDER_STACK_BYTES = 1 << 20

/** Initial memory (`limits::WASM_INITIAL_PAGES`, 16 MiB). */
export const INITIAL_PAGES = 256

/**
 * The shared memory of both instances: 16 MiB to start, the wasm32 cap of 4 GiB as maximum
 * when the browser allows it, else the largest of 2 GiB and 1 GiB (CT-3).
 */
export function engineMemory(): WebAssembly.Memory {
  let last: unknown
  for (const maximum of [65536, 32768, 16384]) {
    try {
      return new WebAssembly.Memory({ initial: INITIAL_PAGES, maximum, shared: true })
    } catch (error) {
      last = error
    }
  }
  throw last
}

/**
 * The WASI functions the module imports (`wasm32-wasip1-threads`). Only `random_get` does
 * something (the prepare side's hash maps seed from it); the render instance calls it only
 * for the warm-up's private channel (#355), where `crypto` may be missing.
 */
export function wasiImports(memory: WebAssembly.Memory) {
  return {
    random_get(ptr: number, len: number) {
      const bytes = new Uint8Array(len)
      // An AudioWorkletGlobalScope has no `crypto`: the render instance's hash maps (the warm-up
      // builds some) then seed from `Math.random`, which is enough for a throwaway channel.
      if (typeof crypto !== 'undefined') crypto.getRandomValues(bytes)
      else for (let i = 0; i < len; i++) bytes[i] = Math.floor(Math.random() * 256)
      new Uint8Array(memory.buffer, ptr, len).set(bytes)
      return 0
    },
    environ_get: () => 0,
    environ_sizes_get: (count: number, size: number) => {
      const view = new DataView(memory.buffer)
      view.setUint32(count, 0, true)
      view.setUint32(size, 0, true)
      return 0
    },
    fd_write: () => 0,
    proc_exit(code: number) {
      throw new Error(`engine exited (${code})`)
    },
    sched_yield: () => 0,
  }
}

/** One instance of the engine module on `memory`. */
export function instantiate(module: WebAssembly.Module, memory: WebAssembly.Memory): EngineExports {
  const instance = new WebAssembly.Instance(module, {
    env: { memory },
    wasi_snapshot_preview1: wasiImports(memory),
  })
  return instance.exports as unknown as EngineExports
}

/** What the prepare instance hands to the render instance. */
export interface RenderSetup {
  channel: number
  /** Top of the render instance's stack (stacks grow down). */
  stackTop: number
  /** The render instance's thread-local block (`__wasm_init_tls`). */
  tls: number
}

/**
 * Creates the channel on the prepare instance (role set first) and the stack and
 * thread-local block of the render instance; throws when the memory is refused.
 */
export function createChannel(prepare: EngineExports, sampleRate: number): RenderSetup {
  prepare.fk_set_role(ROLE_PREPARE)
  const channel = prepare.fk_channel_new(sampleRate)
  const stack = prepare.fk_alloc(RENDER_STACK_BYTES, 16)
  const tlsSize = Number(prepare.__tls_size.value)
  const tls = prepare.fk_alloc(Math.max(tlsSize, 1), Number(prepare.__tls_align.value))
  if (!channel || !stack || !tls) throw new Error('engine: out of memory')
  return { channel, stackTop: stack + RENDER_STACK_BYTES, tls }
}

/**
 * Makes `render` the render instance: its own stack and thread-local block, set before any
 * other call (two instances must never share them), then its role.
 */
export function becomeRenderInstance(render: EngineExports, setup: RenderSetup) {
  render.__stack_pointer.value = setup.stackTop
  render.__wasm_init_tls(setup.tls)
  render.fk_set_role(ROLE_RENDER)
}

/**
 * Warms the render instance's apply path (#355, `fk_warm`): every engine message goes once
 * through a private channel, so that V8 compiles those wasm functions now and not in the first
 * block that applies a message. Call it once, after `becomeRenderInstance` and before any audio.
 * Returns 0 on success. It never throws: a trap (a panic, a refused allocation) is caught and
 * the instance put back as it was (stack pointer, role), since a cold first block is better
 * than no engine; the result is then -1. What a real trap leaves behind is the private channel
 * it never dropped, a leak in the shared heap for the engine's lifetime.
 */
export function warmUp(render: EngineExports, sampleRate: number): number {
  const stack = render.__stack_pointer.value
  try {
    return render.fk_warm(sampleRate)
  } catch {
    render.__stack_pointer.value = stack
    render.fk_set_role(ROLE_RENDER)
    return -1
  }
}

/** A clock in milliseconds and its step in seconds, to time the render (#486). */
export interface RenderClock {
  now: () => number
  step: number
}

/** A block read longer than this (a wall-clock jump of `Date.now()`) counts as 0 s. */
export const MAX_TIMED_BLOCK_MS = 1000

/**
 * The finest clock of the scope (#486): `performance.now()` where it exists, else `Date.now()`
 * (an AudioWorkletGlobalScope in Chromium has no `performance`; its `Date.now()` steps by 1 ms).
 * Its step is measured once, from one tick to the next, in at most `spins` reads (a clock that
 * does not move in them is taken to step by 1 ms). Call it before any audio.
 */
export function renderClock(scope: { performance?: { now(): number } } = globalThis, spins = 1e7): RenderClock {
  const perf = scope.performance
  const now = typeof perf?.now === 'function' ? () => perf.now() : () => Date.now()
  let left = spins
  const next = (from: number) => {
    let t = now()
    while (t === from && --left > 0) t = now()
    return t
  }
  const first = next(now())
  const second = next(first)
  const step = left > 0 && second > first ? (second - first) / 1000 : 0.001
  return { now, step }
}

/**
 * The seconds to report for a block timed from `start` to `end` by a [`RenderClock`]: a
 * negative reading or one over [`MAX_TIMED_BLOCK_MS`] (the wall clock stepped) counts as 0.
 */
export function blockSeconds(start: number, end: number): number {
  const ms = end - start
  return ms >= 0 && ms < MAX_TIMED_BLOCK_MS ? ms / 1000 : 0
}
