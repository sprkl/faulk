// The offline render of the song to a WAV file (issue #297): an instance of the engine module of
// its own, on a memory of its own, never the worklet's and never while it plays. It is fed the
// stream the project core encoded (`ProjectCore.exportJob`), steps the engine faster than real
// time and posts the file's bytes as they are made. The Worker (`export-worker.ts`) runs it; the
// page cancels by terminating the Worker, so nothing here needs to be interruptible.
//
// Audio clips (#647): the render's engine reads their files through its disk planner. Between
// steps this module serves its reads from the project's folder with the executor the live engine
// uses (`DiskExecutor`), and the engine renders a block only once they are served, so the file is
// the song as it plays. A file that cannot be read leaves its clips silent and is named in `done`.
import { DiskExecutor, type Dir, type DiskExports } from '../audio/stream/executor'
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
  /** The stream that sets the engine up. */
  setup: ArrayBuffer
  /** The stream that starts it after the pre-roll. */
  start: ArrayBuffer
  /** The stream that stops it once the content is rendered. */
  stop: ArrayBuffer
  /** The project's folder, where the audio clips' files are read (#647); `null`: none (they are silent). */
  folder: FileSystemDirectoryHandle | null
  /**
   * Measure only (`project.analyze_render`, #665): the render stops once the content is rendered, posts no chunk and
   * answers `done` with the levels of the content. The file's header and bytes are not made.
   */
  analysis?: boolean
}

/** The content's levels (linear amplitude), as the engine measured them (`Offline::measure`). */
export interface ContentLevels {
  /** Frames measured: the content's length. */
  frames: number
  /** By track slot, up to the last occupied one. */
  tracks: { peak_l: number; peak_r: number; rms_l: number; rms_r: number }[]
  master: { peak_l: number; peak_r: number; rms_l: number; rms_r: number }
  /** Frames the output protection silenced for a non-finite sample, over the content. */
  silenced: number
  /** The output protection's peak gain reduction over the content (linear, 0 = never engaged). */
  reduction: number
}

export type FromExport =
  /** Bytes of the file after the header, with how far the render is. */
  | { type: 'chunk'; bytes: ArrayBuffer; rendered: number; body: number; tail: boolean }
  /** The render is complete: the header to put at the start of the file, and what was measured. */
  | {
      type: 'done'
      header: ArrayBuffer
      written: number
      peak: number
      clipped: number
      /** The output protection's peak gain reduction (linear, 0 = never engaged; #402). */
      reduction: number
      /** Frames the output protection silenced for a non-finite sample (a NaN or an infinity; 0 on a clean render, #402). */
      silenced: number
      tailCut: boolean
      /** What the content measured; `null` if the engine reported none. */
      levels?: ContentLevels | null
      /** The media (blob hashes, hex) of the audio clips whose file could not be read: silent in the file (#647). */
      missing: string[]
    }
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
  ex.fk_export_set_start(handle, put(message.start))
  const submitted = ex.fk_export_submit(handle, put(message.setup))
  if (submitted !== 0) throw new ExportFailure(`the engine refused the project (${submitted})`, submitted)
  const disk = new DiskExecutor(diskOf(ex), memory, handle)
  disk.folder(message.folder as unknown as Dir | null)
  for (;;) {
    const phase = ex.fk_export_step(handle, STEP_FRAMES)
    if (phase < 0) throw new ExportFailure(`the render failed (${phase})`, phase)
    if (message.analysis) {
      // A measure writes nothing and ends with the content: the tail is neither written nor measured. It still serves
      // the disk reads its audio clips wait for (below).
      if (phase !== 0) break
    } else {
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
      // Only a chunk with bytes is acknowledged by the page once written: an empty one takes no room.
      if (len > 0) await room?.()
    }
    // The step stopped before a block whose reads are not served: serve them, then step again.
    while (ex.fk_export_stat(handle, 9) > 0) {
      disk.tick()
      // Jobs the engine could not hand over were failed back (it no longer waits): step again.
      if (disk.inFlight.size === 0 && ex.fk_export_stat(handle, 9) === 0) break
      // A wait with no read to make would never end: a failure, not a hang.
      if (disk.inFlight.size === 0) throw new ExportFailure('the render waits for a read it never gave', null)
      await Promise.all([...disk.inFlight])
    }
  }
  const levels = readLevels(ex, handle, memory)
  const at = ex.fk_export_header(handle)
  const header = new Uint8Array(memory.buffer, at, ex.fk_export_header_len(handle)).slice().buffer
  post(
    {
      type: 'done',
      header,
      written: ex.fk_export_stat(handle, 1),
      peak: ex.fk_export_stat(handle, 3),
      clipped: ex.fk_export_stat(handle, 4),
      reduction: ex.fk_export_stat(handle, 7),
      silenced: ex.fk_export_stat(handle, 8),
      tailCut: ex.fk_export_stat(handle, 5) > 0.5,
      levels,
      missing: missingOf(ex, memory, handle),
    },
    [header],
  )
  ex.fk_export_free(handle)
}

/** The export's disk exports as the executor calls them: the export's handle stands for the channel. */
function diskOf(ex: ExportExports): DiskExports {
  return {
    // The render polls its planner itself, with its own clock: the executor only takes the jobs.
    fk_disk_poll: (handle) => ex.fk_export_disk_jobs(handle),
    fk_disk_jobs: (handle) => ex.fk_export_disk_table(handle),
    fk_disk_inbox: (handle, len) => ex.fk_export_disk_inbox(handle, len),
    fk_disk_done: (handle, id, len, fileLen) => ex.fk_export_disk_done(handle, id, len, fileLen),
    fk_disk_failed: (handle, id, code) => ex.fk_export_disk_failed(handle, id, code),
  }
}

/** The media the render could not read, as hex hashes. */
function missingOf(ex: ExportExports, memory: WebAssembly.Memory, handle: number): string[] {
  const count = ex.fk_export_missing(handle)
  const at = ex.fk_export_missing_table(handle)
  const bytes = new Uint8Array(memory.buffer, at, count * 32)
  return Array.from({ length: count }, (_, i) => Array.from(bytes.subarray(i * 32, i * 32 + 32), (b) => b.toString(16).padStart(2, '0')).join(''))
}

/** The levels the engine measured over the content, copied out of its memory; `null` before the content is rendered. */
function readLevels(ex: ExportExports, handle: number, memory: WebAssembly.Memory): ContentLevels | null {
  const frames = ex.fk_export_stat(handle, 12)
  const strips = ex.fk_export_levels_len(handle)
  if (frames < 0 || strips < 1) return null
  const at = ex.fk_export_levels(handle)
  if (!at) return null
  const values = new Float32Array(memory.buffer, at, strips * 4).slice()
  const strip = (i: number) => ({ peak_l: values[i * 4], peak_r: values[i * 4 + 1], rms_l: values[i * 4 + 2], rms_r: values[i * 4 + 3] })
  return {
    frames,
    tracks: Array.from({ length: strips - 1 }, (_, i) => strip(i)),
    master: strip(strips - 1),
    silenced: ex.fk_export_stat(handle, 13),
    reduction: ex.fk_export_stat(handle, 14),
  }
}
