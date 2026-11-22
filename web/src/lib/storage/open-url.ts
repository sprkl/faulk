// Opening a `.faulkpack` hosted somewhere (#497): which links the page fetches, and the download itself.
// The command (`project.open_url`) has checked the link's shape in the core already; the page parses it
// again with the browser's own parser, adds what only it knows (where the page itself runs), and reads
// the bytes with a size guard. The pack's content is checked by the core afterwards (`packImport`), as
// for a file the user picks. Nothing here runs code from the pack.

/** The hosts plain `http` may reach: the computer itself. */
const LOOPBACK = new Set(['localhost', '127.0.0.1', '[::1]'])

/** A download that sends nothing for this long (no answer, no new bytes) is given up. */
export const STALL_MS = 30_000

/** Whether a host (as `URL.hostname` gives it) is this computer. */
export const isLoopback = (hostname: string): boolean => LOOPBACK.has(hostname.toLowerCase())

export type CheckedUrl = { status: 'ok'; url: URL; host: string } | { status: 'refused' }

/**
 * Whether the page fetches `raw`: `https:`, or `http:` to the computer itself when the page is served
 * from it too (`pageHostname`, the page's `location.hostname`: development, the tests, a local preview).
 * A deployed page never reaches into the visitor's own machine through a link. No user name or password.
 */
export function checkOpenUrl(raw: string, pageHostname: string): CheckedUrl {
  let url: URL
  try {
    url = new URL(raw)
  } catch {
    return { status: 'refused' }
  }
  if (url.username !== '' || url.password !== '' || url.hostname === '') return { status: 'refused' }
  const local = url.protocol === 'http:' && isLoopback(url.hostname) && isLoopback(pageHostname)
  if (url.protocol !== 'https:' && !local) return { status: 'refused' }
  return { status: 'ok', url, host: url.host }
}

export type Download = { status: 'ok'; bytes: Uint8Array } | { status: 'unreachable'; detail: string } | { status: 'tooBig' } | { status: 'cancelled' }

/** A download written somewhere as it comes (#682): how many bytes it took, or why it stopped. */
export type Streamed = { status: 'ok'; size: number } | { status: 'unreachable'; detail: string } | { status: 'tooBig' } | { status: 'cancelled' }

/** The most bytes read whole when an answer has no stream (not seen in the supported browsers): the page's own limit. */
const WHOLE_MAX = 192 << 20

export interface DownloadOptions {
  fetch: typeof fetch
  max: number
  pageHostname: string
  stallMs?: number
  /** Stops the download (`cancelled`). */
  signal?: AbortSignal
  /** The bytes received so far, and the length the host announced (`null` when it announced none). */
  onProgress?(received: number, announced: number | null): void
}

/**
 * Downloads a pack into memory (where the browser keeps no projects, the pack's project lives in the tab):
 * `streamPack` into an array, with the same checks.
 */
export async function downloadPack(url: URL, options: DownloadOptions): Promise<Download> {
  const chunks: Uint8Array[] = []
  const streamed = await streamPack(url, { ...options, sink: { write: async (chunk) => void chunks.push(chunk) } })
  if (streamed.status !== 'ok') return streamed
  const bytes = new Uint8Array(streamed.size)
  let at = 0
  for (const chunk of chunks) {
    bytes.set(chunk, at)
    at += chunk.byteLength
  }
  return { status: 'ok', bytes }
}

/**
 * Downloads a pack into `sink`, piece by piece: no cookies or credentials, no referrer, CORS. Refused as `tooBig`
 * before reading when the announced length passes `max`, and while reading as soon as the bytes do (the body is
 * cancelled, so a host that lies about the length or sends no length cannot fill the disk or the memory). A network
 * error, a CORS refusal, an answer that is not a success, a redirect to a link the page would not fetch itself, and a
 * download that stalls for `stallMs` are all `unreachable`; the time the sink takes to write a piece is not a stall.
 * An aborted `signal` is `cancelled`. A sink that fails rejects with its error.
 */
export async function streamPack(url: URL, options: DownloadOptions & { sink: { write(chunk: Uint8Array): Promise<void> } }): Promise<Streamed> {
  const stallMs = options.stallMs ?? STALL_MS
  const abort = new AbortController()
  let timer: ReturnType<typeof setTimeout> | null = null
  // Any progress (the answer, then each chunk) pushes the deadline back: a big pack on a slow link that
  // keeps coming is not cut, a host that goes silent is.
  const alive = () => {
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => abort.abort(new Error(`no data for ${stallMs} ms`)), stallMs)
  }
  const pause = () => {
    if (timer) clearTimeout(timer)
    timer = null
  }
  const cancel = () => abort.abort(new DOMException('cancelled', 'AbortError'))
  if (options.signal?.aborted) return { status: 'cancelled' }
  options.signal?.addEventListener('abort', cancel, { once: true })
  const unreachable = (error: unknown): Streamed =>
    options.signal?.aborted ? { status: 'cancelled' } : { status: 'unreachable', detail: error instanceof Error ? error.message : String(error) }
  alive()
  try {
    let response: Response
    try {
      response = await options.fetch(url.href, {
        credentials: 'omit',
        mode: 'cors',
        redirect: 'follow',
        referrerPolicy: 'no-referrer',
        signal: abort.signal,
      })
    } catch (error) {
      return unreachable(error)
    }
    alive()
    // A redirect may land anywhere: the place the bytes come from passes the same check as the link.
    if (response.url && checkOpenUrl(response.url, options.pageHostname).status !== 'ok') {
      void response.body?.cancel().catch(() => undefined)
      return unreachable(new Error(`redirected to a link the page does not open: ${response.url}`))
    }
    if (!response.ok) {
      void response.body?.cancel().catch(() => undefined)
      return unreachable(new Error(`HTTP ${response.status}`))
    }
    const header = response.headers.get('content-length')
    const announced = header === null ? NaN : Number(header)
    if (Number.isFinite(announced) && announced > options.max) {
      void response.body?.cancel().catch(() => undefined)
      return { status: 'tooBig' }
    }
    const length = Number.isFinite(announced) ? announced : null
    if (!response.body) {
      // No stream to count: read whole, then the same limit, and only for a download into memory (whose limit is the
      // page's own): a pack too big for memory needs a stream.
      if (options.max > WHOLE_MAX) return unreachable(new Error('the answer cannot be read in pieces'))
      const whole = new Uint8Array(await response.arrayBuffer())
      if (whole.byteLength > options.max) return { status: 'tooBig' }
      pause()
      await options.sink.write(whole)
      options.onProgress?.(whole.byteLength, length)
      return { status: 'ok', size: whole.byteLength }
    }
    const reader = response.body.getReader()
    let total = 0
    for (;;) {
      let next: ReadableStreamReadResult<Uint8Array>
      try {
        next = await reader.read()
      } catch (error) {
        return unreachable(error)
      }
      if (next.done) break
      total += next.value.byteLength
      if (total > options.max) {
        await reader.cancel().catch(() => undefined)
        return { status: 'tooBig' }
      }
      // The sink's time is the page's, not the host's.
      pause()
      try {
        await options.sink.write(next.value)
      } catch (error) {
        await reader.cancel().catch(() => undefined)
        throw error
      }
      if (options.signal?.aborted) {
        await reader.cancel().catch(() => undefined)
        return { status: 'cancelled' }
      }
      alive()
      options.onProgress?.(total, length)
    }
    return { status: 'ok', size: total }
  } finally {
    if (timer) clearTimeout(timer)
    options.signal?.removeEventListener('abort', cancel)
  }
}

/** The name an opened pack is listed under before its project tells its own: the file's name in the link. */
export function packNameOf(url: URL): string {
  return lastSegmentOf(url).replace(/\.faulkpack$/i, '') || 'pack'
}

/** The last part of a link's path, percent-decoded (as it is when it does not decode); empty for none. */
export function lastSegmentOf(url: URL): string {
  const last = url.pathname.split('/').filter(Boolean).pop() ?? ''
  try {
    return decodeURIComponent(last)
  } catch {
    return last
  }
}
