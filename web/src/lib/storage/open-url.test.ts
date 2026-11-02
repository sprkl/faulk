// Which links the page fetches for `project.open_url`, and the download's guards (#497).
import { describe, expect, it, vi } from 'vitest'
import { checkOpenUrl, downloadPack, packNameOf } from './open-url'

const ok = (raw: string, page = 'faulk.app') => checkOpenUrl(raw, page).status === 'ok'

describe('checkOpenUrl', () => {
  it('takes https links and names their host', () => {
    expect(checkOpenUrl('https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/a/project.faulkpack', 'faulk.app')).toMatchObject({
      status: 'ok',
      host: 'cdn.jsdelivr.net',
    })
    expect(checkOpenUrl('https://example.org:8443/a.faulkpack', 'faulk.app')).toMatchObject({ host: 'example.org:8443' })
  })

  it('refuses other schemes, credentials and what is no link', () => {
    for (const raw of [
      '',
      'not a link',
      '/relative/song.faulkpack',
      'ftp://example.org/a.faulkpack',
      'file:///C:/a.faulkpack',
      'javascript:alert(1)',
      'data:application/zip;base64,UEsDBA==',
      'blob:https://faulk.app/1234',
      'https://user:secret@example.org/a.faulkpack',
      'https://user@example.org/a.faulkpack',
    ]) {
      expect(ok(raw), raw).toBe(false)
    }
  })

  it('refuses plain http outside this computer, and on this computer unless the page runs here too', () => {
    expect(ok('http://example.org/a.faulkpack')).toBe(false)
    expect(ok('http://example.org/a.faulkpack', 'localhost')).toBe(false)
    // A deployed page never reaches into the visitor's own machine.
    expect(ok('http://localhost:5497/a.faulkpack', 'faulk.app')).toBe(false)
    expect(ok('http://127.0.0.1:9/a', 'faulk.app')).toBe(false)
    // A page on this computer (development, tests, a local preview) may.
    expect(ok('http://localhost:5497/a.faulkpack', 'localhost')).toBe(true)
    expect(ok('http://127.0.0.1:9/a', '127.0.0.1')).toBe(true)
    expect(ok('http://[::1]:9/a', 'localhost')).toBe(true)
    // Look-alikes of this computer stay outside.
    expect(ok('http://localhost.example.org/a', 'localhost')).toBe(false)
    expect(ok('http://localhost@example.org/a', 'localhost')).toBe(false)
    expect(ok('http://example.org\\@localhost/a', 'localhost')).toBe(false)
  })
})

/** A `fetch` that answers `body` in chunks, with the headers given. */
function serving(chunks: Uint8Array[], init: ResponseInit & { url?: string } = {}) {
  let cancelled = false
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(chunk)
      controller.close()
    },
    cancel() {
      cancelled = true
    },
  })
  const fetch = vi.fn(async () => {
    const response = new Response(stream, init)
    if (init.url) Object.defineProperty(response, 'url', { value: init.url })
    return response
  }) as unknown as typeof globalThis.fetch & ReturnType<typeof vi.fn>
  return { fetch, cancelled: () => cancelled }
}

const URL_ = new URL('https://example.org/song.faulkpack')
const base = { max: 10, pageHostname: 'faulk.app' }

describe('downloadPack', () => {
  it('downloads without cookies, credentials or referrer, through CORS', async () => {
    const { fetch } = serving([new Uint8Array([1, 2, 3]), new Uint8Array([4])])
    expect(await downloadPack(URL_, { ...base, fetch })).toEqual({ status: 'ok', bytes: new Uint8Array([1, 2, 3, 4]) })
    expect(fetch).toHaveBeenCalledWith(URL_.href, expect.objectContaining({ credentials: 'omit', mode: 'cors', referrerPolicy: 'no-referrer' }))
  })

  it('says unreachable for a network or CORS failure and for an answer that is not a success', async () => {
    const failing = vi.fn(async () => {
      throw new TypeError('Failed to fetch')
    }) as unknown as typeof fetch
    expect(await downloadPack(URL_, { ...base, fetch: failing })).toEqual({ status: 'unreachable', detail: 'Failed to fetch' })
    const { fetch } = serving([new Uint8Array([1])], { status: 404 })
    expect(await downloadPack(URL_, { ...base, fetch })).toEqual({ status: 'unreachable', detail: 'HTTP 404' })
  })

  it('refuses a pack announced too big before reading it', async () => {
    const { fetch, cancelled } = serving([new Uint8Array(4)], { headers: { 'content-length': '11' } })
    expect(await downloadPack(URL_, { ...base, fetch })).toEqual({ status: 'tooBig' })
    expect(cancelled()).toBe(true)
  })

  it('stops reading as soon as the bytes pass the limit, whatever the host announced', async () => {
    const lying = serving([new Uint8Array(6), new Uint8Array(6), new Uint8Array(6)], { headers: { 'content-length': '3' } })
    expect(await downloadPack(URL_, { ...base, fetch: lying.fetch })).toEqual({ status: 'tooBig' })
    expect(lying.cancelled()).toBe(true)
    // Exactly the limit passes; one byte more does not.
    expect((await downloadPack(URL_, { ...base, fetch: serving([new Uint8Array(10)]).fetch })).status).toBe('ok')
    expect((await downloadPack(URL_, { ...base, fetch: serving([new Uint8Array(10), new Uint8Array(1)]).fetch })).status).toBe('tooBig')
  })

  it('refuses a redirect to a link the page would not fetch itself', async () => {
    const { fetch } = serving([new Uint8Array(1)], { url: 'http://example.org/elsewhere.faulkpack' })
    expect(await downloadPack(URL_, { ...base, fetch })).toMatchObject({ status: 'unreachable' })
    const fine = serving([new Uint8Array(1)], { url: 'https://mirror.example.net/song.faulkpack' })
    expect((await downloadPack(URL_, { ...base, fetch: fine.fetch })).status).toBe('ok')
  })

  it('gives up a host that goes silent, but not one that keeps sending', async () => {
    vi.useFakeTimers()
    try {
      const silent = vi.fn(
        (_url: string, init: RequestInit) =>
          new Promise<Response>((_resolve, reject) => init.signal?.addEventListener('abort', () => reject(init.signal?.reason as Error))),
      ) as unknown as typeof fetch
      const pending = downloadPack(URL_, { ...base, fetch: silent, stallMs: 1000 })
      await vi.advanceTimersByTimeAsync(1000)
      expect(await pending).toEqual({ status: 'unreachable', detail: 'no data for 1000 ms' })

      // Five chunks 600 ms apart: 3 s in all, never 1 s without data.
      let sent = 0
      const slow = vi.fn(async () => {
        const body = new ReadableStream<Uint8Array>({
          async pull(controller) {
            await new Promise((resolve) => setTimeout(resolve, 600))
            if (sent++ < 5) controller.enqueue(new Uint8Array(1))
            else controller.close()
          },
        })
        return new Response(body)
      }) as unknown as typeof fetch
      const slowDownload = downloadPack(URL_, { ...base, fetch: slow, stallMs: 1000 })
      await vi.advanceTimersByTimeAsync(5000)
      expect(await slowDownload).toEqual({ status: 'ok', bytes: new Uint8Array(5) })
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('packNameOf', () => {
  it('names a pack after its file', () => {
    expect(packNameOf(new URL('https://a.org/x/My%20song.faulkpack?v=1'))).toBe('My song')
    expect(packNameOf(new URL('https://a.org/'))).toBe('pack')
    expect(packNameOf(new URL('https://a.org/%E0%A4%A.faulkpack'))).toBe('%E0%A4%A')
  })
})
