// An agent's audio file from a link (#664): its name, and the download's guards. The guards are `streamPack`'s
// (tested in `storage/open-url.test.ts`); these tests attack them through the audio path, with its own cap.
import { afterEach, describe, expect, it, vi } from 'vitest'
import { MAX_AUDIO_URL_BYTES } from '../../generated/protocol'
import { audioNameOf, downloadAudio } from './from-url'

/** A `fetch` that answers `chunks`, with the headers given; `url` sets where the answer says it came from. */
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

const LINK = new URL('https://example.org/loops/kick.wav')
const base = { pageHostname: 'faulk.app' }

afterEach(() => {
  vi.useRealTimers()
})

describe('audioNameOf', () => {
  it('names the file after the last part of the path, decoded, without the query or the fragment', () => {
    expect(audioNameOf(new URL('https://example.org/loops/kick%2001.wav?v=2#x'))).toBe('kick 01.wav')
    expect(audioNameOf(new URL('https://example.org/a/b/'))).toBe('b')
    expect(audioNameOf(new URL('https://example.org/get.php?id=3'))).toBe('get.php')
  })

  it('falls back to "audio", and keeps no separator, control character or bad escape in the name', () => {
    expect(audioNameOf(new URL('https://example.org/'))).toBe('audio')
    expect(audioNameOf(new URL('https://example.org/%2F%2F'))).toBe('audio')
    expect(audioNameOf(new URL('https://example.org/a%2Fb%5Cc.wav'))).toBe('a b c.wav')
    expect(audioNameOf(new URL('https://example.org/a%0Ab%00.wav'))).toBe('a b .wav')
    expect(audioNameOf(new URL('https://example.org/100%25.wav'))).toBe('100%.wav')
    expect(audioNameOf(new URL('https://example.org/bad%E0%A4%A.wav'))).toBe('bad%E0%A4%A.wav')
  })

  it('cannot pass for the question’s own words: no quotation mark, no reordering mark, a name of 80 characters at most', () => {
    // A name that would close its quotes and name another host.
    const spoof = audioNameOf(new URL('https://evil.example/kick.wav%E2%80%9D%20from%20faulk.app%20%22%27%C2%BB%60'))
    expect(spoof).toBe('kick.wav from faulk.app')
    // Right-to-left override: "kick‮vaw.exe" would read as "kickexe.wav".
    expect(audioNameOf(new URL('https://example.org/kick%E2%80%AEvaw.exe'))).toBe('kick vaw.exe')
    expect(audioNameOf(new URL('https://example.org/a%E2%81%A6b%E2%80%8Fc'))).toBe('a b c')
    const long = audioNameOf(new URL(`https://example.org/${'a'.repeat(300)}.wav`))
    expect(Array.from(long)).toHaveLength(80)
    expect(long.endsWith('…')).toBe(true)
  })
})

describe('downloadAudio', () => {
  it('downloads without cookies, credentials or referrer, through CORS, into a file of no type', async () => {
    const { fetch } = serving([new Uint8Array([1, 2]), new Uint8Array([3])], { headers: { 'content-type': 'text/html' } })
    const got = await downloadAudio(LINK, 'kick.wav', { ...base, fetch })
    if (got.status !== 'ok') throw new Error(JSON.stringify(got))
    expect(fetch).toHaveBeenCalledWith(LINK.href, expect.objectContaining({ credentials: 'omit', mode: 'cors', referrerPolicy: 'no-referrer' }))
    expect(got.file.name).toBe('kick.wav')
    // The host's type is not taken: the decoder reads the content.
    expect(got.file.type).toBe('')
    expect(new Uint8Array(await got.file.arrayBuffer())).toEqual(new Uint8Array([1, 2, 3]))
  })

  it('is capped at MAX_AUDIO_URL_BYTES: announced over it, it is refused before reading', async () => {
    const { fetch, cancelled } = serving([new Uint8Array(4)], { headers: { 'content-length': String(MAX_AUDIO_URL_BYTES + 1) } })
    expect(await downloadAudio(LINK, 'kick.wav', { ...base, fetch })).toEqual({ status: 'tooBig' })
    expect(cancelled()).toBe(true)
  })

  it('cuts a body that grows past the cap, whether its length is announced short or not at all', async () => {
    for (const headers of [{ 'content-length': '2' }, {}] as Record<string, string>[]) {
      const { fetch, cancelled } = serving([new Uint8Array(3), new Uint8Array(3), new Uint8Array(3)], { headers })
      expect(await downloadAudio(LINK, 'kick.wav', { ...base, fetch, max: 5 })).toEqual({ status: 'tooBig' })
      expect(cancelled()).toBe(true)
    }
  })

  it('refuses a redirect to a link the page would not fetch, and an answer that is not a success', async () => {
    for (const url of ['http://example.org/kick.wav', 'http://localhost/kick.wav', 'https://u:p@example.org/kick.wav']) {
      const { fetch } = serving([new Uint8Array(1)], { url })
      expect(await downloadAudio(LINK, 'kick.wav', { ...base, fetch }), url).toMatchObject({ status: 'unreachable' })
    }
    const { fetch } = serving([], { status: 404 })
    expect(await downloadAudio(LINK, 'kick.wav', { ...base, fetch })).toEqual({ status: 'unreachable', detail: 'HTTP 404' })
    const refused = vi.fn(async () => Promise.reject(new TypeError('Failed to fetch'))) as unknown as typeof globalThis.fetch
    expect(await downloadAudio(LINK, 'kick.wav', { ...base, fetch: refused })).toEqual({ status: 'unreachable', detail: 'Failed to fetch' })
  })

  it('gives up a host that stops sending, and stops on a cancel', async () => {
    vi.useFakeTimers()
    const silent = vi.fn(
      (_: string, init: RequestInit) =>
        new Promise<Response>((_, reject) => init.signal?.addEventListener('abort', () => reject(init.signal?.reason as Error), { once: true })),
    ) as unknown as typeof globalThis.fetch
    const stalled = downloadAudio(LINK, 'kick.wav', { ...base, fetch: silent, stallMs: 1000 })
    await vi.advanceTimersByTimeAsync(1001)
    expect(await stalled).toMatchObject({ status: 'unreachable', detail: 'no data for 1000 ms' })
    const stop = new AbortController()
    const cancelled = downloadAudio(LINK, 'kick.wav', { ...base, fetch: silent, signal: stop.signal })
    stop.abort()
    expect(await cancelled).toEqual({ status: 'cancelled' })
  })
})
