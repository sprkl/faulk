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
// draft), which is a project folder like any other.

test.describe.configure({ timeout: 180_000 })

const FIXTURES = fileURLToPath(new URL('../../../crates/audio-import/tests/fixtures/', import.meta.url))
const fixture = (name: string) => `${FIXTURES}${name}`
const sha256 = (path: string) => createHash('sha256').update(readFileSync(path)).digest('hex')

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

/** Whether the page, after a reload, has its project back (the draft reopens by itself). */
const reopened = (page: Page) =>
  page.evaluate(() => {
    const view = (window as unknown as { __faulkView?: () => unknown }).__faulkView
    return typeof view === 'function' && view() !== null && view() !== undefined
  })

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
  // The cache goes (a new machine, a cleaned folder): the next opening makes it again.
  await page.evaluate(async (path) => {
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
  }, cache)
  expect((await opfsFiles(page))[cache]).toBeUndefined()
  await page.reload()
  await expect.poll(() => reopened(page), { timeout: 30_000 }).toBe(true)
  const report = await page.evaluate(() => (window as unknown as { __faulkAudioCache(): Promise<{ made: string[] } | null> }).__faulkAudioCache())
  expect(report?.made).toEqual([hash])
  expect((await opfsFiles(page))[cache]).toBe(32 + 22_050 * 2 * 4)
  expect(errors).toEqual([])
})

test('a file that is not audio, or in a format Faulk does not read, is refused by name and nothing is kept', async ({ page }) => {
  const { errors } = watch(page)
  const track = await audioTrack(page)
  await choose(page, fixture('not-audio.txt'))
  await expect(page.getByTestId('audio-import-failure')).toHaveAttribute('data-code', 'unsupported_format')
  await expect(page.getByTestId('audio-import-failure')).toContainText('not-audio.txt is not an audio file Faulk can read.')
  await choose(page, fixture('sine.opus'))
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

// The helper's session, as a stand-in (helper.spec.ts has the full one): enough for the page to switch to Boost.
const CODE = '12345678'
const TOKEN = 'e2e-token'
const text = (data: RawData) => (Array.isArray(data) ? Buffer.concat(data) : data instanceof ArrayBuffer ? Buffer.from(data) : data).toString('utf8')

test('a one-hour file: the progress shows as it goes, and in Boost no byte of it crosses to the helper @full', async ({ page }) => {
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
    } else ws.close()
  })
  try {
    const { errors } = watch(page)
    const track = await audioTrack(page)
    await page.getByTestId('engine-mode').click()
    await page.getByRole('textbox', { name: /code/i }).fill('1234 5678')
    await page.getByRole('button', { name: 'Pair', exact: true }).click()
    await engineUp(page, 'Boost')
    await page.keyboard.press('Escape')
    await expect.poll(() => frames.length).toBeGreaterThan(0)
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
    const run = Buffer.from(Array.from({ length: 32 }, (_, i) => (i % 2 ? 0x3c : 0x5a)))
    for (const frame of sent) expect(frame.includes(run)).toBe(false)
    expect(errors).toEqual([])
  } finally {
    for (const client of server.clients) client.terminate()
    await new Promise((resolve) => server.close(resolve))
  }
})
