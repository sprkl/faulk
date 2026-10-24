// The browser's side of an export (issue #297): the file dialog and the file it writes, the
// download where there is no dialog, the Worker and the engine module.
import wasmUrl from '../engine/wasm/engine.wasm?url'
import type { ExportDeps, ExportSink, ExportWorkerPort, SinkChoice } from './audio-export.svelte'

/** The File System Access API's save dialog (Chromium; not in `lib.dom`). */
type SaveWindow = {
  showSaveFilePicker?: (options: {
    suggestedName: string
    startIn?: FileSystemHandle | 'music' | 'documents' | 'downloads'
    types: { description: string; accept: Record<string, string[]> }[]
  }) => Promise<FileSystemFileHandle>
}

const canChoose = () => typeof window !== 'undefined' && typeof (window as SaveWindow).showSaveFilePicker === 'function'

/** A file the user chose: the header's room is kept at the start, the bytes follow, the header is written last. */
function fileSink(name: string, writable: FileSystemWritableFileStream): ExportSink {
  return {
    name,
    kind: 'file',
    write: (bytes) => writable.write(bytes as Uint8Array<ArrayBuffer>),
    async finish(header) {
      await writable.seek(0)
      await writable.write(header as Uint8Array<ArrayBuffer>)
      await writable.close()
    },
    abort: () => writable.abort().catch(() => undefined),
  }
}

/** A browser without the save dialog: the file is kept in memory and handed over at the end. */
function downloadSink(name: string, headerBytes: number): ExportSink {
  const parts: BlobPart[] = [new Uint8Array(headerBytes)]
  return {
    name,
    kind: 'download',
    async write(bytes) {
      parts.push(bytes as Uint8Array<ArrayBuffer>)
    },
    async finish(header) {
      parts[0] = header as Uint8Array<ArrayBuffer>
      const url = URL.createObjectURL(new Blob(parts, { type: 'audio/wav' }))
      const link = document.createElement('a')
      link.href = url
      link.download = name
      link.click()
      setTimeout(() => URL.revokeObjectURL(url), 60_000)
    },
    async abort() {
      parts.length = 0
    },
  }
}

export const browserExportDeps: ExportDeps = {
  get canChoose() {
    return canChoose()
  },
  async pick(name, headerBytes, startIn): Promise<SinkChoice> {
    if (!canChoose()) return { status: 'chosen', sink: downloadSink(name, headerBytes) }
    try {
      const handle = await (window as SaveWindow).showSaveFilePicker!({
        suggestedName: name,
        startIn: startIn ?? 'music',
        types: [{ description: 'WAV audio', accept: { 'audio/wav': ['.wav'] } }],
      })
      const writable = await handle.createWritable()
      // The header is only known at the end: its room is kept now.
      await writable.write(new Uint8Array(headerBytes))
      return { status: 'chosen', sink: fileSink(handle.name, writable) }
    } catch (error) {
      if (error instanceof DOMException && error.name === 'AbortError') return { status: 'cancelled' }
      console.error('export: the file could not be opened', error)
      return { status: 'refused' }
    }
  },
  spawn(): ExportWorkerPort {
    return new Worker(new URL('./export-worker.ts', import.meta.url), { type: 'module' })
  },
  engineModule: () => fetch(wasmUrl).then((response) => response.arrayBuffer()),
  now: () => performance.now(),
}
