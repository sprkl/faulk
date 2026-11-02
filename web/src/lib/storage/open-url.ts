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

export type Download = { status: 'ok'; bytes: Uint8Array } | { status: 'unreachable'; detail: string } | { status: 'tooBig' }

/**
 * Downloads a pack: no cookies or credentials, no referrer, CORS. Refused as `tooBig` before reading when
 * the announced length passes `max`, and while reading as soon as the bytes do (the body is cancelled, so
 * a host that lies about the length or sends no length cannot fill the memory). A network error, a CORS
 * refusal, an answer that is not a success, a redirect to a link the page would not fetch itself, and a
 * download that stalls for `stallMs` are all `unreachable`.
 */
export async function downloadPack(url: URL, options: { fetch: typeof fetch; max: number; pageHostname: string; stallMs?: number }): Promise<Download> {
  const stallMs = options.stallMs ?? STALL_MS
  const abort = new AbortController()
  let timer: ReturnType<typeof setTimeout> | null = null
  // Any progress (the answer, then each chunk) pushes the deadline back: a big pack on a slow link that
  // keeps coming is not cut, a host that goes silent is.
  const alive = () => {
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => abort.abort(new Error(`no data for ${stallMs} ms`)), stallMs)
  }
  const unreachable = (error: unknown): Download => ({ status: 'unreachable', detail: error instanceof Error ? error.message : String(error) })
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
    const announced = Number(response.headers.get('content-length'))
    if (Number.isFinite(announced) && announced > options.max) {
      void response.body?.cancel().catch(() => undefined)
      return { status: 'tooBig' }
    }
    if (!response.body) {
      // No stream to count (not seen in the supported browsers): read whole, then the same limit.
      const whole = new Uint8Array(await response.arrayBuffer())
      return whole.byteLength > options.max ? { status: 'tooBig' } : { status: 'ok', bytes: whole }
    }
    const reader = response.body.getReader()
    const chunks: Uint8Array[] = []
    let total = 0
    for (;;) {
      let next: ReadableStreamReadResult<Uint8Array>
      try {
        next = await reader.read()
      } catch (error) {
        return unreachable(error)
      }
      if (next.done) break
      alive()
      total += next.value.byteLength
      if (total > options.max) {
        await reader.cancel().catch(() => undefined)
        return { status: 'tooBig' }
      }
      chunks.push(next.value)
    }
    const bytes = new Uint8Array(total)
    let at = 0
    for (const chunk of chunks) {
      bytes.set(chunk, at)
      at += chunk.byteLength
    }
    return { status: 'ok', bytes }
  } finally {
    if (timer) clearTimeout(timer)
  }
}

/** The name an opened pack is listed under before its project tells its own: the file's name in the link. */
export function packNameOf(url: URL): string {
  const last = url.pathname.split('/').filter(Boolean).pop() ?? ''
  let name: string
  try {
    name = decodeURIComponent(last)
  } catch {
    name = last
  }
  return name.replace(/\.faulkpack$/i, '') || 'pack'
}
