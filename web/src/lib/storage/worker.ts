// The project store's Worker (SV-2): every read and write of a project folder runs here, off the
// page's main thread, so a save of a big project never holds up the interface or the engine's
// feeding. One `FolderBackend` per folder the page opened; the page talks to it by `rpc.ts`. The
// audio import (AU-1) runs here too: it writes the folder, and it needs `FileReaderSync`, which only
// a Worker has. It reports progress and hears a cancel; between its steps the other calls run. So
// does the `.faulkpack` import (#682), which writes the pack's blobs as it reads them.
import { ImportError } from '../audio/import/abi'
import { classify, FolderBackend } from './backend'
import { packInWorker } from './pack-worker'
import type { Reply, Request } from './rpc'
import { STORE_METHODS } from './rpc'

const backends = new Map<number, FolderBackend>()

/** The imports that run, by request id: a cancel aborts them. */
const running = new Map<number, AbortController>()

/** The pack exports that wait for the page to have written a piece, by request id: the page's word, or an abort, ends the wait. */
const packWaits = new Map<number, () => void>()

const post = (reply: Reply, transfer: ArrayBuffer[] = []) => (self as unknown as { postMessage(m: Reply, t: ArrayBuffer[]): void }).postMessage(reply, transfer)

/** The buffers a value carries, to hand over instead of copying. */
function buffersOf(value: unknown): ArrayBuffer[] {
  const found: ArrayBuffer[] = []
  const visit = (v: unknown) => {
    if (v instanceof Uint8Array) found.push(v.buffer as ArrayBuffer)
    else if (Array.isArray(v)) v.forEach(visit)
    else if (v && typeof v === 'object') Object.values(v).forEach(visit)
  }
  visit(value)
  return found
}

function backendOf(key: number): FolderBackend {
  const backend = backends.get(key)
  if (!backend) throw new Error(`no folder under key ${key}`)
  return backend
}

async function handle(request: Request): Promise<unknown> {
  if (request.op === 'open') {
    backends.set(request.key, new FolderBackend(request.handle))
    return request.handle.name
  }
  if (request.op === 'close') {
    backends.get(request.key)?.close()
    backends.delete(request.key)
    return null
  }
  if (request.op === 'cancel') {
    running.get(request.target)?.abort()
    return null
  }
  if (request.op === 'packAck') {
    // The page wrote the piece (or could not: then it stops the export).
    if (request.failed) running.get(request.target)?.abort()
    packWaits.get(request.target)?.()
    return null
  }
  if (request.op === 'pack') {
    const backend = backendOf(request.key)
    const control = new AbortController()
    running.set(request.id, control)
    const release = () => packWaits.get(request.id)?.()
    control.signal.addEventListener('abort', release)
    try {
      return await packInWorker(request.entries, control.signal, {
        read: (path, start, end) => backend.read(path, start, end),
        send: async (chunk) => {
          if (control.signal.aborted) return
          const waiting = new Promise<void>((resolve) => packWaits.set(request.id, resolve))
          post({ id: request.id, chunk }, [chunk.buffer as ArrayBuffer])
          await waiting
        },
        progress: (fraction) => post({ id: request.id, progress: fraction }),
      })
    } finally {
      control.signal.removeEventListener('abort', release)
      packWaits.delete(request.id)
      running.delete(request.id)
    }
  }
  if (request.op === 'unpack') {
    const backend = backendOf(request.key)
    const control = new AbortController()
    running.set(request.id, control)
    try {
      return await backend.unpack(request.source, {
        onProgress: (fraction) => post({ id: request.id, progress: fraction }),
        signal: control.signal,
        textOnly: request.textOnly,
      })
    } finally {
      running.delete(request.id)
    }
  }
  if (request.op === 'import' || request.op === 'audioCache') {
    const backend = backendOf(request.key)
    const control = new AbortController()
    running.set(request.id, control)
    const progress = (fraction: number) => post({ id: request.id, progress: fraction })
    try {
      if (request.op === 'import') return await backend.importAudio(request.file, progress, control.signal)
      return await backend.ensureAudioCache(request.media, progress, control.signal)
    } finally {
      running.delete(request.id)
    }
  }
  const backend = backendOf(request.key)
  if (!STORE_METHODS.includes(request.method)) throw new Error(`not a store method: ${request.method}`)
  if (request.method === 'copyBlobsFrom') {
    const source = backends.get(request.args[0] as number)
    if (!source) throw new Error('no source folder')
    return backend.copyBlobsFrom(source)
  }
  // eslint-disable-next-line @typescript-eslint/unbound-method -- read here, called on the next line with `this` bound to the backend
  const call = backend[request.method] as (...args: unknown[]) => Promise<unknown>
  return call.apply(backend, request.args)
}

self.onmessage = async (event: MessageEvent<Request>) => {
  const request = event.data
  let reply: Reply
  let transfer: ArrayBuffer[] = []
  try {
    const value = await handle(request)
    // A cancel has no answer: the import it stops answers.
    if (request.op === 'cancel' || request.op === 'packAck') return
    transfer = buffersOf(value)
    reply = { id: request.id, ok: true, value }
  } catch (error) {
    if (error instanceof ImportError) {
      reply = { id: request.id, ok: false, code: 'io', message: error.message, importCode: error.code }
    } else {
      const failure = classify(error)
      reply = { id: request.id, ok: false, code: failure.code, message: failure.message }
    }
  }
  post(reply, transfer)
}
