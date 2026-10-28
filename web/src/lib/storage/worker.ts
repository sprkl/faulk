// The project store's Worker (SV-2): every read and write of a project folder runs here, off the
// page's main thread, so a save of a big project never holds up the interface or the engine's
// feeding. One `FolderBackend` per folder the page opened; the page talks to it by `rpc.ts`.
import { classify, FolderBackend } from './backend'
import type { Reply, Request } from './rpc'
import { STORE_METHODS } from './rpc'

const backends = new Map<number, FolderBackend>()

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

async function handle(request: Request): Promise<unknown> {
  if (request.op === 'open') {
    backends.set(request.key, new FolderBackend(request.handle))
    return request.handle.name
  }
  const backend = backends.get(request.key)
  if (request.op === 'close') {
    backends.delete(request.key)
    return null
  }
  if (!backend) throw new Error(`no folder under key ${request.key}`)
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
    transfer = buffersOf(value)
    reply = { id: request.id, ok: true, value }
  } catch (error) {
    const failure = classify(error)
    reply = { id: request.id, ok: false, code: failure.code, message: failure.message }
  }
  ;(self as unknown as { postMessage(m: Reply, t: ArrayBuffer[]): void }).postMessage(reply, transfer)
}
