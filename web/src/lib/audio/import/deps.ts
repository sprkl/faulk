// What the import pipeline needs of where it runs: the WebAssembly module (fetched and compiled
// once, a new instance after a trap) and a synchronous reader of a file (`FileReaderSync`, which a
// Worker has and the page does not: there the import is refused as unavailable).
import { ImportError, ImportModule, type ReadSync } from './abi'
import type { PipelineDeps } from './pipeline'
import importUrl from './wasm/import.wasm?url'

let compiled: Promise<WebAssembly.Module> | null = null

/**
 * A new instance of the import module for one job (an import, one cache to make): an instance holds
 * one job's state, and the Worker runs an import and the caches of an opening at the same time, their
 * steps interleaved. Compiled once, from the page's own file.
 */
export async function importModule(): Promise<ImportModule> {
  compiled ??= fetch(importUrl)
    .then((r) => r.arrayBuffer())
    .then((b) => WebAssembly.compile(b))
  // A failed fetch is tried again on the next import.
  compiled.catch(() => (compiled = null))
  return new ImportModule(await compiled)
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
