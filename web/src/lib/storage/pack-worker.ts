// The `.faulkpack` export in the store's Worker (#685): the zip is deflated here, off the page's main thread, which a
// 1 MiB piece of media used to hold for tens of milliseconds at a time. The Worker has its own instance of the project
// module for the `pw_pack_*` session (no project in it), reads the blobs from the folder itself and hands each piece of
// zip to the page, which writes it where the user chose; the next piece waits for the page's word, so a slow disk
// slows the export and memory stays at one piece.
import { coreModule, exportPack, packExports, type FolderEntry, type PackExport } from '../project/core'

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
