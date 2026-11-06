import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { WebSocketServer, type RawData, type WebSocket } from 'ws'
import { PROTOCOL_VERSION } from '../../src/lib/generated/protocol'
import { engineUp, HELPER_HOST, HELPER_PORT, watch } from './support'

// AU-1 (#110): importing an audio file. The user picks a file in an audio track's Clips panel; the page's
// project store Worker hashes it, decodes it with the WebAssembly build of crates/audio-import, keeps its
// bytes in `blobs/<sha256>` and, for a file that is not read directly, its decoded PCM in
// `.faulk/cache/pcm/`; `clip.import_audio` makes the clip. Here in the browser's own storage (the OPFS
// draft), which is a project folder like any other. AU-2 (#111): the import also writes the file's
// waveform peaks to `.faulk/cache/peaks/`, which the Clips panel draws; an opening makes missing ones again.

test.describe.configure({ timeout: 180_000 })

const FIXTURES = fileURLToPath(new URL('../../../crates/audio-import/tests/fixtures/', import.meta.url))
const fixture = (name: string) => `${FIXTURES}${name}`
const sha256 = (path: string) => createHash('sha256').update(readFileSync(path)).digest('hex')
/** What the native build records of each fixture (`peaks_sha256`: the waveform peaks it computes). */
const expected = JSON.parse(readFileSync(fixture('expected.json'), 'utf8')) as Record<string, { peaks_sha256?: string }>

/** The size of the peaks file of a media: a 32-byte header, then four levels of 2 bytes a channel a peak (crates/audio-import `peaks.rs`). */
const peaksSize = (frames: number, channels: number) => 32 + [256, 1024, 4096, 16_384].reduce((n, block) => n + Math.ceil(frames / block) * 2 * channels, 0)
const peaksPath = (hash: string) => `.faulk/cache/peaks/${hash}.peaks`

type View = {
  project: {
    tracks: Record<
      string,
      { kind: string; clips: Record<string, { name: string; content: { audio?: { media: { hash: string; frames: number; name: string } } } }> }
    >
  }
}
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun(c), command)
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)

/** Every file of the page's browser projects, by path below `faulk/browser/<dir>/`, with its size. */
const opfsFiles = (page: Page) =>
  page.evaluate(async () => {
    const out: Record<string, number> = {}
    const walk = async (dir: FileSystemDirectoryHandle, base: string) => {
      for await (const entry of (dir as unknown as { values(): AsyncIterable<FileSystemHandle> }).values()) {
        const path = base ? `${base}/${entry.name}` : entry.name
        if (entry.kind === 'directory') await walk(entry as FileSystemDirectoryHandle, path)
        else out[path] = (await (entry as FileSystemFileHandle).getFile()).size
      }
    }
    try {
      const browser = await (await (await navigator.storage.getDirectory()).getDirectoryHandle('faulk')).getDirectoryHandle('browser')
      for await (const project of (browser as unknown as { values(): AsyncIterable<FileSystemHandle> }).values()) {
        if (project.kind === 'directory') await walk(project as FileSystemDirectoryHandle, '')
      }
    } catch {
      // No browser project yet.
    }
    return out
  })

/** The SHA-256 of a file of the page's browser projects (below `faulk/browser/<dir>/`), or `null`. */
const opfsDigest = (page: Page, path: string) =>
  page.evaluate(async (path) => {
    const browser = await (await (await navigator.storage.getDirectory()).getDirectoryHandle('faulk')).getDirectoryHandle('browser')
    for await (const project of (browser as unknown as { values(): AsyncIterable<FileSystemDirectoryHandle> }).values()) {
      try {
        const parts = path.split('/')
        let dir: FileSystemDirectoryHandle = project
        for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part)
        const bytes = await (await (await dir.getFileHandle(parts.at(-1)!)).getFile()).arrayBuffer()
        return [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map((b) => b.toString(16).padStart(2, '0')).join('')
      } catch {
        // Another project's directory.
      }
    }
    return null
  }, path)

/** Removes a file of the page's browser projects (a new machine, a cleaned folder, a project from before AU-2). */
const opfsRemove = (page: Page, path: string) =>
  page.evaluate(async (path) => {
    const browser = await (await (await navigator.storage.getDirectory()).getDirectoryHandle('faulk')).getDirectoryHandle('browser')
    for await (const project of (browser as unknown as { values(): AsyncIterable<FileSystemDirectoryHandle> }).values()) {
      const parts = path.split('/')
      let dir: FileSystemDirectoryHandle = project
      try {
        for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part)
        await dir.removeEntry(parts.at(-1)!)
      } catch {
        // Another project's directory.
      }
    }
  }, path)

/** Whether the page, after a reload, has its project back (the draft reopens by itself). */
const reopened = (page: Page) =>
  page.evaluate(() => {
    const view = (window as unknown as { __faulkView?: () => unknown }).__faulkView
    return typeof view === 'function' && view() !== null && view() !== undefined
  })

/**
 * The report of the caches the last opening made again. The page's hook waits for the work of an attach that is
 * running, and answers `null` while none has begun it (the project is back before the storage is attached):
 * ask until it reports, on the condition and not on a delay (#707).
 */
async function audioCacheReport(page: Page, timeout = 60_000): Promise<{ made: string[]; peaks: string[] }> {
  const deadline = Date.now() + timeout
  for (;;) {
    const report = await page.evaluate(() =>
      (window as unknown as { __faulkAudioCache(): Promise<{ made: string[]; peaks: string[] } | null> }).__faulkAudioCache(),
    )
    if (report) return report
    if (Date.now() > deadline) throw new Error('the audio cache report never came')
    await page.waitForTimeout(50)
  }
}

/** Starts audio and makes an audio track, selected, with the Clips panel open. Returns the track. */
async function audioTrack(page: Page): Promise<string> {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page)
  test.skip((await hooks(page)) !== 'function', 'the command hooks exist in the dev server only')
  await run(page, { type: 'track.create', kind: 'audio' })
  const track = Object.entries((await view(page)).project.tracks).find(([, t]) => t.kind === 'audio')![0]
  await run(page, { type: 'view.select_track', track })
  await page.getByTestId('panel-clips').click()
  await expect(page.getByTestId('audio-import-button')).toBeVisible()
  return track
}

/** Clicks Import audio… and hands the file chooser these files. */
async function choose(page: Page, ...paths: string[]) {
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('audio-import-button').click()
  await (await chooser).setFiles(paths)
}

const audioClips = async (page: Page, track: string) => Object.values((await view(page)).project.tracks[track]?.clips ?? {}).filter((c) => c.content.audio)

test('a WAV chosen in the Clips panel becomes a clip, its bytes kept once in blobs/, back after a reload; undo keeps the file', async ({ page }) => {
  const { errors } = watch(page)
  const track = await audioTrack(page)
  const hash = sha256(fixture('sine-s16.wav'))
  await choose(page, fixture('sine-s16.wav'))
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)
  await expect(page.getByTestId('pool-clip')).toContainText('sine-s16')
  const [clip] = await audioClips(page, track)
  expect(clip!.content.audio!.media).toMatchObject({ hash, frames: 22_050, name: 'sine-s16.wav' })
  // The draft holds the file under its SHA-256 (the same as `sha256sum`), and no PCM cache: a WAV is read directly.
  await expect.poll(async () => Object.keys(await opfsFiles(page)).filter((p) => p.startsWith('blobs/'))).toEqual([`blobs/${hash}`])
  expect((await opfsFiles(page))[`blobs/${hash}`]).toBe(readFileSync(fixture('sine-s16.wav')).length)
  expect(Object.keys(await opfsFiles(page)).filter((p) => p.startsWith('.faulk/cache/pcm'))).toEqual([])
  // Its waveform peaks (AU-2): the native build's bytes, drawn in the clip's row.
  expect((await opfsFiles(page))[peaksPath(hash)]).toBe(peaksSize(22_050, 2))
  expect(await opfsDigest(page, peaksPath(hash))).toBe(expected['sine-s16.wav']!.peaks_sha256)
  const wave = page.getByTestId('pool-waveform').getByTestId('waveform')
  await expect(wave).toHaveAttribute('data-state', 'ready')
  await expect.poll(async () => Number(await wave.getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await expect(wave).not.toHaveAttribute('data-level', '')
  // The same file again: a second clip, still one blob.
  await choose(page, fixture('sine-s16.wav'))
  await expect(page.getByTestId('pool-clip')).toHaveCount(2)
  expect(Object.keys(await opfsFiles(page)).filter((p) => p.startsWith('blobs/'))).toEqual([`blobs/${hash}`])
  // Undo takes the clip away and leaves the file.
  await run(page, { type: 'history.undo' })
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)
  expect(Object.keys(await opfsFiles(page))).toContain(`blobs/${hash}`)
  // The draft is written, the page reloads, the project comes back with its clip.
  await expect.poll(async () => Object.keys(await opfsFiles(page)).some((p) => p.includes('/clips/'))).toBe(true)
  await page.waitForTimeout(2000)
  await page.reload()
  await expect.poll(() => reopened(page), { timeout: 30_000 }).toBe(true)
  await expect.poll(async () => (await audioClips(page, track)).length, { timeout: 30_000 }).toBe(1)
  expect(errors).toEqual([])
})

test('an MP3 gets its decoded PCM in the cache, and a cache deleted is made again at the next opening', async ({ page }) => {
  const { errors } = watch(page)
  const track = await audioTrack(page)
  const hash = sha256(fixture('sine.mp3'))
  await choose(page, fixture('sine.mp3'))
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)
  const cache = `.faulk/cache/pcm/${hash}.pcm`
  // 32 bytes of header, then 22 050 stereo frames of f32.
  await expect.poll(async () => (await opfsFiles(page))[cache]).toBe(32 + 22_050 * 2 * 4)
  expect((await audioClips(page, track))[0]!.content.audio!.media.frames).toBe(22_050)
  await expect.poll(async () => Object.keys(await opfsFiles(page)).some((p) => p.includes('/clips/'))).toBe(true)
  await page.waitForTimeout(2000)
  expect((await opfsFiles(page))[peaksPath(hash)]).toBe(peaksSize(22_050, 2))
  // The caches go (a new machine, a cleaned folder; no peaks at all for a project from before AU-2): the next opening makes them again.
  await opfsRemove(page, cache)
  await opfsRemove(page, peaksPath(hash))
  expect((await opfsFiles(page))[cache]).toBeUndefined()
  await page.reload()
  await expect.poll(() => reopened(page), { timeout: 30_000 }).toBe(true)
  const report = await audioCacheReport(page)
  expect(report?.made).toEqual([hash])
  expect(report?.peaks).toEqual([hash])
  expect((await opfsFiles(page))[cache]).toBe(32 + 22_050 * 2 * 4)
  expect(await opfsDigest(page, peaksPath(hash))).toBe(expected['sine.mp3']!.peaks_sha256)
  // The clip's row draws them.
  await page.getByTestId('panel-clips').click()
  await expect(page.getByTestId('pool-waveform').getByTestId('waveform')).toHaveAttribute('data-state', 'ready')
  expect(errors).toEqual([])
})

test('Opus in Ogg and in WebM, and a browser recording, become clips of their exact length, decoded as the native build decodes them (#609)', async ({
  page,
}) => {
  const { errors } = watch(page)
  const track = await audioTrack(page)
  const files = { 'sine.opus': 24_000, 'sine.webm': 24_000, 'recorded.webm': 46_080 }
  await choose(page, ...Object.keys(files).map(fixture))
  await expect(page.getByTestId('pool-clip')).toHaveCount(3)
  const clips = await audioClips(page, track)
  for (const [name, frames] of Object.entries(files)) {
    const hash = sha256(fixture(name))
    // The pre-skip and the padding are cut: the source's half second at 48 kHz, or the recording's 16 packets of 60 ms.
    expect(clips.find((c) => c.content.audio!.media.name === name)!.content.audio!.media).toMatchObject({ hash, frames })
    await expect.poll(async () => (await opfsFiles(page))[`.faulk/cache/pcm/${hash}.pcm`]).toBe(32 + frames * 2 * 4)
    expect(await opfsDigest(page, peaksPath(hash))).toBe(expected[name]!.peaks_sha256)
  }
  expect(errors).toEqual([])
})

test('a file that is not audio, or in a format Faulk does not read, is refused by name and nothing is kept', async ({ page }) => {
  const { errors } = watch(page)
  const track = await audioTrack(page)
  await choose(page, fixture('not-audio.txt'))
  await expect(page.getByTestId('audio-import-failure')).toHaveAttribute('data-code', 'unsupported_format')
  await expect(page.getByTestId('audio-import-failure')).toContainText('not-audio.txt is not an audio file Faulk can read.')
  // An Opus stream whose channels Faulk does not model (mapping family 3, ambisonics).
  const ambisonic = readFileSync(fixture('sine.webm'))
  ambisonic[ambisonic.indexOf('OpusHead') + 18] = 3
  const refused = page.waitForEvent('filechooser')
  await page.getByTestId('audio-import-button').click()
  await (await refused).setFiles({ name: 'ambisonic.webm', mimeType: 'audio/webm', buffer: ambisonic })
  await expect(page.getByTestId('audio-import-failure')).toHaveAttribute('data-code', 'unsupported_codec')
  // A damaged WAV (cut in the middle of its samples).
  const wav = readFileSync(fixture('sine-s16.wav'))
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('audio-import-button').click()
  await (await chooser).setFiles({ name: 'cut.wav', mimeType: 'audio/wav', buffer: wav.subarray(0, wav.length / 2 + 1) })
  await expect(page.getByTestId('audio-import-failure')).toHaveAttribute('data-code', 'corrupt')
  expect(await audioClips(page, track)).toEqual([])
  expect(Object.keys(await opfsFiles(page)).filter((p) => p.startsWith('blobs/') || p.startsWith('.faulk/tmp/') || p.startsWith('.faulk/cache/'))).toEqual([])
  expect(errors).toEqual([])
})

test("an agent's audio file from a link (#664): the user is asked first, nothing reaches the host before Allow, then it imports as a chosen file", async ({
  page,
}) => {
  const { errors } = watch(page)
  const track = await audioTrack(page)
  const hash = sha256(fixture('sine-s16.wav'))
  const link = 'https://audio.example/loops/sine-s16.wav?v=1'
  // The host, as Playwright serves it: any origin may read it (CORS); every request it gets is counted.
  let body: Buffer | string = readFileSync(fixture('sine-s16.wav'))
  // The host has a cookie in this browser: the download must not send it.
  await page.context().addCookies([{ name: 'session', value: 'secret', domain: 'audio.example', path: '/', secure: true, sameSite: 'None' }])
  const hits: { method: string; cookie?: string; referer?: string }[] = []
  await page.route('https://audio.example/**', async (route) => {
    const headers = route.request().headers()
    hits.push({ method: route.request().method(), cookie: headers['cookie'], referer: headers['referer'] })
    await route.fulfill({ body, headers: { 'access-control-allow-origin': '*', 'content-type': 'audio/wav', 'cross-origin-resource-policy': 'cross-origin' } })
  })
  const ask = (url = link) =>
    page.evaluate(
      ([track, url]) => {
        const w = window as unknown as { __faulkAgent(c: object, author: string): Promise<unknown>; __answer?: Promise<unknown> }
        w.__answer = w.__faulkAgent({ type: 'clip.import_audio_url', track, url }, 'agent:claude-code')
      },
      [track, url],
    )
  const answer = () => page.evaluate(() => (window as unknown as { __answer: Promise<unknown> }).__answer)
  const dialog = page.getByTestId('agent-confirm')

  // Refused: nothing was ever sent to the host.
  await ask()
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('heading')).toHaveText('claude-code wants to download an audio file from audio.example')
  await expect(dialog).toContainText('File: sine-s16.wav.')
  await expect(dialog).toContainText('up to 192 MB')
  await page.getByTestId('agent-confirm-refuse').click()
  expect(await answer()).toMatchObject({ status: 'refused', error: { code: 'confirmation.declined' } })
  expect(hits).toEqual([])
  await expect(page.getByTestId('pool-clip')).toHaveCount(0)

  // Allowed: one request, without cookies or referrer; the clip is the agent's, the file kept under its SHA-256.
  await ask()
  await page.getByTestId('agent-confirm-allow').click()
  expect(await answer()).toMatchObject({ status: 'applied', output: { media: hash } })
  expect(hits).toEqual([{ method: 'GET', cookie: undefined, referer: undefined }])
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)
  await expect(page.getByTestId('pool-clip')).toContainText('sine-s16')
  expect((await audioClips(page, track))[0]!.content.audio!.media).toMatchObject({ hash, frames: 22_050, name: 'sine-s16.wav' })
  const history = await page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: { name: string; author: string }[] } }).__faulkHistory())
  expect(history.undo.at(-1)).toMatchObject({ name: 'clip.import_audio', author: 'agent:claude-code' })
  await expect.poll(async () => Object.keys(await opfsFiles(page))).toContain(`blobs/${hash}`)

  // The same call again (an agent that heard `page.timeout`): the clip already made, no question, no request.
  expect(
    await page.evaluate(
      ([track, url]) => (window as unknown as { __faulkAgent(c: object): Promise<unknown> }).__faulkAgent({ type: 'clip.import_audio_url', track, url }),
      [track, link],
    ),
  ).toMatchObject({
    status: 'applied',
    output: { media: hash },
  })
  expect(hits).toHaveLength(1)
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)

  // A web page served under an audio name and type: the decoder refuses it, the agent hears why, no clip is made.
  body = '<!doctype html><p>Not found</p>'.repeat(40)
  await ask('https://audio.example/loops/missing.wav')
  await page.getByTestId('agent-confirm-allow').click()
  expect(await answer()).toMatchObject({ status: 'refused', error: { code: 'import.unsupported_format' } })
  await expect(page.getByTestId('audio-import-failure')).toHaveAttribute('data-code', 'unsupported_format')
  await expect(page.getByTestId('pool-clip')).toHaveCount(1)
  expect(errors).toEqual([])
})

// The helper's session, as a stand-in (helper.spec.ts has the full one): enough for the page to switch to Boost.
const CODE = '12345678'
const TOKEN = 'e2e-token'
const text = (data: RawData) => (Array.isArray(data) ? Buffer.concat(data) : data instanceof ArrayBuffer ? Buffer.from(data) : data).toString('utf8')

/** A stand-in helper on the e2e port: records every frame the page sends on its session. */
async function standInHelper() {
  const server = new WebSocketServer({ host: HELPER_HOST, port: HELPER_PORT })
  await new Promise<void>((resolve, reject) => {
    server.once('listening', resolve)
    server.once('error', (error) => reject(new Error(`port ${HELPER_PORT} is taken (a helper running?): ${String(error)}`)))
  })
  /** Every frame the page sent the helper on its session, as bytes. */
  const frames: Buffer[] = []
  server.on('connection', (ws: WebSocket, req) => {
    const url = new URL(req.url ?? '/', `http://${HELPER_HOST}`)
    const send = (message: object) => ws.send(JSON.stringify(message))
    if (url.pathname === '/probe') send({ type: 'probe', helper: 'faulk-helper', protocol: PROTOCOL_VERSION, audio_output: true })
    else if (url.pathname === '/pair') ws.on('message', (data) => JSON.parse(text(data)).code === CODE && send({ type: 'paired', token: TOKEN }))
    else if (url.pathname === '/session' && url.searchParams.get('token') === TOKEN) {
      send({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000, estimated_latency_ms: 5, epoch: 40, capabilities: { midi: true, disk: false } })
      send({ type: 'midi.inputs', inputs: [], selected: null })
      ws.on('message', (data) => frames.push(Array.isArray(data) ? Buffer.concat(data) : Buffer.from(data as Buffer)))
    } else if (url.pathname === '/control' && url.searchParams.get('token') === TOKEN) send({ type: 'control.hello', protocol: PROTOCOL_VERSION })
    else ws.close()
  })
  const close = async () => {
    for (const client of server.clients) client.terminate()
    await new Promise((resolve) => server.close(resolve))
  }
  return { frames, close }
}

/** Pairs the page with the stand-in helper and waits for Boost. */
async function toBoost(page: Page, frames: Buffer[]) {
  await page.getByTestId('engine-mode').click()
  await page.getByRole('textbox', { name: /code/i }).fill('1234 5678')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await engineUp(page, 'Boost')
  await page.keyboard.press('Escape')
  await expect.poll(() => frames.length).toBeGreaterThan(0)
}

test('the same file gets the same waveform peaks in Web and in Boost, those of the native build, and none of them crosses to the helper', async ({ page }) => {
  const helper = await standInHelper()
  try {
    const { errors } = watch(page)
    const track = await audioTrack(page)
    const hash = sha256(fixture('sine.mp3'))
    // Web: the import writes the peaks.
    await choose(page, fixture('sine.mp3'))
    await expect(page.getByTestId('pool-clip')).toHaveCount(1)
    await expect.poll(() => opfsDigest(page, peaksPath(hash))).toBe(expected['sine.mp3']!.peaks_sha256)
    // Boost: the same file again, its peaks file removed first so that this import writes it anew.
    await toBoost(page, helper.frames)
    await opfsRemove(page, peaksPath(hash))
    const before = helper.frames.length
    await choose(page, fixture('sine.mp3'))
    await expect(page.getByTestId('pool-clip')).toHaveCount(2)
    await expect.poll(() => opfsDigest(page, peaksPath(hash))).toBe(expected['sine.mp3']!.peaks_sha256)
    // The peaks are the page's: no frame to the helper carries them, neither their header's magic nor a run of their level-0 bytes.
    const file = Buffer.from(
      await page.evaluate(async (path) => {
        const browser = await (await (await navigator.storage.getDirectory()).getDirectoryHandle('faulk')).getDirectoryHandle('browser')
        for await (const project of (browser as unknown as { values(): AsyncIterable<FileSystemDirectoryHandle> }).values()) {
          try {
            let dir: FileSystemDirectoryHandle = project
            const parts = path.split('/')
            for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part)
            return [...new Uint8Array(await (await (await dir.getFileHandle(parts.at(-1)!)).getFile()).arrayBuffer())]
          } catch {
            // Another project's directory.
          }
        }
        return []
      }, peaksPath(hash)),
    )
    expect(file.length).toBe(peaksSize(22_050, 2))
    for (const run of [file.subarray(0, 8), file.subarray(32, 64), file.subarray(200, 232)]) {
      for (const frame of helper.frames.slice(before)) expect(frame.includes(run)).toBe(false)
    }
    await expect(page.getByTestId('pool-waveform').first().getByTestId('waveform')).toHaveAttribute('data-state', 'ready')
    void track
    expect(errors).toEqual([])
  } finally {
    await helper.close()
  }
})

test('a one-hour file: the progress shows as it goes, and in Boost no byte of it crosses to the helper @full', async ({ page }) => {
  const { frames, close } = await standInHelper()
  try {
    const { errors } = watch(page)
    const track = await audioTrack(page)
    await toBoost(page, frames)
    const before = frames.length
    // One hour of mono 16-bit audio at 16 kHz (115 MB), built in the page: a WAV header then the same second
    // of samples 3 600 times. Its samples carry a run no engine message would: 0x5a, 0x3c repeated.
    const SECONDS = 3600
    const RATE = 16_000
    const imported = page.evaluate(
      async ({ track, seconds, rate }) => {
        const second = new Uint8Array(rate * 2)
        for (let i = 0; i < second.length; i += 2) {
          second[i] = 0x5a
          second[i + 1] = 0x3c
        }
        const data = seconds * second.length
        const header = new DataView(new ArrayBuffer(44))
        const tag = (at: number, s: string) => [...s].forEach((c, i) => header.setUint8(at + i, c.charCodeAt(0)))
        tag(0, 'RIFF')
        header.setUint32(4, 36 + data, true)
        tag(8, 'WAVEfmt ')
        header.setUint32(16, 16, true)
        header.setUint16(20, 1, true)
        header.setUint16(22, 1, true)
        header.setUint32(24, rate, true)
        header.setUint32(28, rate * 2, true)
        header.setUint16(32, 2, true)
        header.setUint16(34, 16, true)
        tag(36, 'data')
        header.setUint32(40, data, true)
        const file = new File([header.buffer, ...Array.from({ length: seconds }, () => second)], 'one-hour.wav', { type: 'audio/wav' })
        const started = performance.now()
        const hashes = await (window as unknown as { __faulkImportAudio(t: string, f: File[]): Promise<string[]> }).__faulkImportAudio(track, [file])
        return { hashes, ms: performance.now() - started, size: file.size }
      },
      { track, seconds: SECONDS, rate: RATE },
    )
    // The progress, as the panel shows it, until the import ends.
    const seen = new Set<number>()
    const progress = page.getByTestId('audio-import-progress')
    await expect(progress).toBeVisible({ timeout: 30_000 })
    let done = false
    void imported.finally(() => (done = true))
    while (!done) {
      const fraction = await progress.getAttribute('data-fraction', { timeout: 1000 }).catch(() => null)
      if (fraction !== null) seen.add(Number(fraction))
      await page.waitForTimeout(100)
    }
    const { hashes, ms, size } = await imported
    const values = [...seen].sort((a, b) => a - b)
    console.log(
      `one-hour import: ${(size / 1e6).toFixed(1)} MB in ${(ms / 1000).toFixed(1)} s, ${values.length} progress values shown from ${values[0]} to ${values.at(-1)}`,
    )
    expect(hashes).toHaveLength(1)
    expect(values.length).toBeGreaterThan(5)
    expect(values.at(-1)!).toBeGreaterThan(0.9)
    const [clip] = await audioClips(page, track)
    expect(clip!.content.audio!.media.frames).toBe(SECONDS * RATE)
    // Nothing of the file reached the helper: what crossed is engine messages and JSON, a few bytes, with no run of the samples.
    const sent = frames.slice(before)
    const bytes = sent.reduce((n, f) => n + f.length, 0)
    console.log(`one-hour import in Boost: ${sent.length} frames, ${bytes} bytes to the helper`)
    expect(bytes).toBeLessThan(64 * 1024)
    const pattern = Buffer.from(Array.from({ length: 32 }, (_, i) => (i % 2 ? 0x3c : 0x5a)))
    for (const frame of sent) expect(frame.includes(pattern)).toBe(false)
    // Its waveform peaks (AU-2): about 45 KB a minute at the finest level in stereo at 48 kHz; here mono at
    // 16 kHz, so 62.5 blocks a second of 2 bytes, 7.5 KB a minute, and the clip's row draws them.
    const hash = hashes[0]!
    const files = await opfsFiles(page)
    expect(files[peaksPath(hash)]).toBe(peaksSize(SECONDS * RATE, 1))
    console.log(`one-hour peaks: ${files[peaksPath(hash)]} bytes (${Math.ceil((SECONDS * RATE) / 256) * 2} at the finest level)`)
    await expect(page.getByTestId('pool-waveform').getByTestId('waveform')).toHaveAttribute('data-state', 'ready')
    // Gone (a project from before AU-2): the next opening decodes the hour again for them, in the background,
    // while the page keeps answering its commands.
    await opfsRemove(page, peaksPath(hash))
    await expect.poll(async () => Object.keys(await opfsFiles(page)).some((p) => p.includes('/clips/'))).toBe(true)
    await page.waitForTimeout(2000)
    const reloaded = performance.now()
    await page.reload()
    await expect.poll(() => reopened(page), { timeout: 60_000 }).toBe(true)
    const answers: number[] = []
    let made = false
    const remade = audioCacheReport(page).then((report) => ((made = true), report))
    while (!made) {
      const start = performance.now()
      await page.evaluate(() => (window as unknown as { __faulkView(): unknown }).__faulkView())
      await run(page, { type: 'view.select_track', track })
      answers.push(performance.now() - start)
      await page.waitForTimeout(200)
    }
    const report = await remade
    const regenMs = performance.now() - reloaded
    console.log(
      `one-hour peaks made again at the opening: ${(regenMs / 1000).toFixed(1)} s from the reload; ${answers.length} commands meanwhile, slowest answer ${Math.max(...answers).toFixed(0)} ms`,
    )
    expect(report?.peaks).toEqual([hash])
    expect((await opfsFiles(page))[peaksPath(hash)]).toBe(peaksSize(SECONDS * RATE, 1))
    expect(Math.max(...answers)).toBeLessThan(1000)
    expect(errors).toEqual([])
  } finally {
    await close()
  }
})
