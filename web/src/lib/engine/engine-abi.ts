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
  fk_left(channel: number): number
  fk_right(channel: number): number
  fk_feedback(channel: number): number
  fk_feedback_len(channel: number): number
  fk_host_counter(channel: number, which: number): number
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
 * something (the prepare side's hash maps seed from it); the render instance calls none.
 */
export function wasiImports(memory: WebAssembly.Memory) {
  return {
    random_get(ptr: number, len: number) {
      const bytes = new Uint8Array(len)
      crypto.getRandomValues(bytes)
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
