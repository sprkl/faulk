// The `.faulkpack` export in the store's Worker (#685): the zip is deflated here, off the page's main thread, which a
// 1 MiB piece of media used to hold for tens of milliseconds at a time. The Worker has its own instance of the project
// module for the `pw_pack_*` session (no project in it), reads the blobs from the folder itself and hands each piece of
// zip to the page, which writes it where the user chose; the next piece waits for the page's word, so a slow disk
// slows the export and memory stays at one piece. The import runs here too (#682): the Worker reads the pack the user
// chose (or the one downloaded into the folder) in the ranges the core asks for and writes each blob as it comes.
import { coreModule, exportPack, packExports, unpackPack, type FolderEntry, type PackExport, type PackUnpack } from '../project/core'
import type { FileWriter } from './types'

/** What the Worker gives the export: the folder to read, and the page to hand zip pieces to. */
export interface PackWorkerDeps {
  /** Bytes `[start, end)` of a file of the folder. */
  read(path: string, start: number, end: number): Promise<Uint8Array>
  /** Hands a piece of zip to the page; settles once the page has written it. */
  send(chunk: Uint8Array): Promise<void>
  progress(fraction: number): void
  /** The compiled project module (`coreModule`; the bytes of the file in tests). */
  module?: () => Promise<WebAssembly.Module>
}

/** Writes the pack of `entries` through `deps`; `cancelled` at the first piece boundary after `signal` aborts. */
export async function packInWorker(entries: FolderEntry[], signal: AbortSignal, deps: PackWorkerDeps): Promise<PackExport> {
  if (signal.aborted) return { status: 'cancelled' }
  const exports = packExports(await (deps.module ?? coreModule)())
  return exportPack(exports, entries, {
    read: (path, start, end) => deps.read(path, start, end),
    sink: { write: (chunk) => deps.send(chunk) },
    signal,
    onProgress: (f) => deps.progress(f),
  })
}

/** What the Worker gives an import (#682): where the blobs go, and the page to tell how far it is. */
export interface UnpackWorkerDeps {
  /** Opens a file of the folder to write in pieces. */
  open(path: string): Promise<FileWriter>
  progress(fraction: number): void
  /** The compiled project module (`coreModule`; the bytes of the file in tests). */
  module?: () => Promise<WebAssembly.Module>
}

/**
 * Imports the pack `source` into a folder through `deps` (`unpackPack` in the Worker's own instance of the
 * module): the pack is read in the ranges the core asks for, each blob written as it comes. A blob cut short
 * (a failure, a cancel) is aborted, never left under its name. `cancelled` at the next piece after `signal` aborts.
 */
export async function unpackInWorker(source: Blob, signal: AbortSignal, deps: UnpackWorkerDeps, textOnly = false): Promise<PackUnpack> {
  if (signal.aborted) return { status: 'cancelled' }
  const exports = packExports(await (deps.module ?? coreModule)())
  let open: FileWriter | null = null
  try {
    return await unpackPack(exports, {
      size: source.size,
      read: async (start, end) => new Uint8Array(await source.slice(start, end).arrayBuffer()),
      sink: {
        start: async (path) => {
          open = await deps.open(path)
        },
        write: (chunk) => (open ? open.write(chunk) : Promise.reject(new Error('no blob is open'))),
        end: async () => {
          const done = open
          open = null
          await done?.close()
        },
      },
      signal,
      onProgress: (f) => deps.progress(f),
      textOnly,
    })
  } finally {
    const left: FileWriter | null = open
    if (left) await (left as FileWriter).abort().catch(() => undefined)
    // For measurements (`pack-import.spec.ts`): the most memory an import's instance took (a module's memory only grows).
    const scope = globalThis as { __faulkUnpackMemory?: number }
    scope.__faulkUnpackMemory = Math.max(scope.__faulkUnpackMemory ?? 0, exports.memory.buffer.byteLength)
  }
}
