// What the import pipeline needs of where it runs: the WebAssembly module (fetched and compiled
// once, a new instance after a trap) and a synchronous reader of a file (`FileReaderSync`, which a
// Worker has and the page does not: there the import is refused as unavailable).
import { ImportError, ImportModule, type ReadSync } from './abi'
import type { PipelineDeps } from './pipeline'
import importUrl from './wasm/import.wasm?url'

let compiled: Promise<WebAssembly.Module> | null = null
let instance: ImportModule | null = null

/** A working instance of the import module, compiled from `bytes` (tests) or fetched once. */
export async function importModule(bytes?: BufferSource): Promise<ImportModule> {
  if (instance && !instance.broken) return instance
  compiled ??= bytes
    ? WebAssembly.compile(bytes)
    : fetch(importUrl)
        .then((r) => r.arrayBuffer())
        .then((b) => WebAssembly.compile(b))
  // A failed fetch is tried again on the next import.
  compiled.catch(() => (compiled = null))
  instance = new ImportModule(await compiled)
  return instance
}

type ReaderSyncCtor = new () => { readAsArrayBuffer(blob: Blob): ArrayBuffer }

/** A synchronous reader of `blob` by ranges. */
export async function syncReader(blob: Blob): Promise<ReadSync> {
  const Ctor = (globalThis as { FileReaderSync?: ReaderSyncCtor }).FileReaderSync
  if (!Ctor) throw new ImportError('unavailable', 'importing audio needs a Worker (FileReaderSync)')
  const reader = new Ctor()
  return (offset, length) => new Uint8Array(reader.readAsArrayBuffer(blob.slice(offset, offset + length)))
}

/** The pipeline's dependencies where it runs in the page's Worker. */
export function workerDeps(onProgress?: (fraction: number) => void, signal?: AbortSignal): PipelineDeps {
  return { module: () => importModule(), readerOf: syncReader, onProgress, signal }
}
