// The offline render of the song to a WAV file (issue #297): an instance of the engine module of
// its own, on a memory of its own, never the worklet's and never while it plays. It is fed the
// stream the project core encoded (`ProjectCore.exportJob`), steps the engine faster than real
// time and posts the file's bytes as they are made. The Worker (`export-worker.ts`) runs it; the
// page cancels by terminating the Worker, so nothing here needs to be interruptible.
import { engineMemory, instantiate, ROLE_PREPARE, type ExportExports } from '../engine/engine-abi'

/** What the page sends: to start a render, then one `ack` per chunk it has written. */
export type ToExport = ToStart | { type: 'ack' }

export interface ToStart {
  type: 'start'
  /** The engine module (`engine.wasm`). */
  module: ArrayBuffer
  sampleRate: number
  /** 16, 24 or 32. */
  bits: number
  /** Frames of the content, before the tail. */
  bodyFrames: number
  /** The stream that sets the engine up and starts it. */
  setup: ArrayBuffer
  /** The stream that stops it once the content is rendered. */
  stop: ArrayBuffer
}

export type FromExport =
  /** Bytes of the file after the header, with how far the render is. */
  | { type: 'chunk'; bytes: ArrayBuffer; rendered: number; body: number; tail: boolean }
  /** The render is complete: the header to put at the start of the file, and what was measured. */
  | { type: 'done'; header: ArrayBuffer; written: number; peak: number; clipped: number; tailCut: boolean }
  | { type: 'error'; message: string; code: number | null }

/** Frames per step: about a third of a second of audio, so progress moves and chunks stay small. */
export const STEP_FRAMES = 16_384

export type Post = (message: FromExport, transfer?: Transferable[]) => void

/** Chunks the Worker may have sent that the page has not written yet (about 3 s of audio each 8). */
export const CHUNKS_IN_FLIGHT = 8

class ExportFailure extends Error {
  constructor(
    message: string,
    readonly code: number | null,
  ) {
    super(message)
  }
}

/**
 * Renders the export, posting its chunks and then `done`; posts `error` instead when it cannot.
 * `room` resolves when the page has written enough of what was sent for more to go: a render far
 * faster than a disk would otherwise pile the whole file up in memory.
 */
export async function renderExport(message: ToStart, post: Post, room?: () => Promise<void>) {
  try {
    await run(message, post, room)
  } catch (error) {
    const code = error instanceof ExportFailure ? error.code : null
    post({ type: 'error', message: error instanceof Error ? error.message : String(error), code })
  }
}

async function run(message: ToStart, post: Post, room?: () => Promise<void>) {
  const memory = engineMemory()
  const ex = instantiate(new WebAssembly.Module(message.module), memory) as ExportExports
  ex.fk_set_role(ROLE_PREPARE)
  const bodyLo = message.bodyFrames % 2 ** 32
  const bodyHi = Math.floor(message.bodyFrames / 2 ** 32)
  const handle = ex.fk_export_new(message.sampleRate, message.bits, bodyLo, bodyHi)
  if (!handle) throw new ExportFailure('the engine refused the export', null)
  const put = (bytes: ArrayBuffer): number => {
    const at = ex.fk_export_inbox(handle, bytes.byteLength)
    if (!at) throw new ExportFailure('out of memory', null)
    new Uint8Array(memory.buffer, at, bytes.byteLength).set(new Uint8Array(bytes))
    return bytes.byteLength
  }
  ex.fk_export_set_stop(handle, put(message.stop))
  const submitted = ex.fk_export_submit(handle, put(message.setup))
  if (submitted !== 0) throw new ExportFailure(`the engine refused the project (${submitted})`, submitted)
  for (;;) {
    const phase = ex.fk_export_step(handle, STEP_FRAMES)
    if (phase < 0) throw new ExportFailure(`the render failed (${phase})`, phase)
    const len = ex.fk_export_pcm_len(handle)
    // Copied out of the engine's memory: the next step reuses it, and a shared buffer cannot be transferred.
    const bytes = len > 0 ? new Uint8Array(memory.buffer, ex.fk_export_pcm(handle), len).slice().buffer : new ArrayBuffer(0)
    post(
      {
        type: 'chunk',
        bytes,
        rendered: ex.fk_export_stat(handle, 0),
        body: ex.fk_export_stat(handle, 2),
        tail: phase !== 0,
      },
      [bytes],
    )
    if (phase === 2) break
    await room?.()
  }
  const at = ex.fk_export_header(handle)
  const header = new Uint8Array(memory.buffer, at, ex.fk_export_header_len(handle)).slice().buffer
  post(
    {
      type: 'done',
      header,
      written: ex.fk_export_stat(handle, 1),
      peak: ex.fk_export_stat(handle, 3),
      clipped: ex.fk_export_stat(handle, 4),
      tailCut: ex.fk_export_stat(handle, 5) > 0.5,
    },
    [header],
  )
  ex.fk_export_free(handle)
}
