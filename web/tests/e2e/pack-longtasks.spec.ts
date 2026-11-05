import { writeFileSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #685: the main thread stays free while a `.faulkpack` is exported. A project with a large audio file (generated
// here, `FAULK_PACK_MIB` MiB of noise, 90 by default, never committed) is exported to a stand-in for the file dialog;
// the Long Tasks of the page and the gaps of a 5 ms timer are recorded during the export, and the pack must be whole.

test.describe.configure({ timeout: 900_000 })

const WAV_BYTES = (Number(process.env.FAULK_PACK_MIB) || 90) << 20

/** A stereo 16-bit 44.1 kHz WAV of noise, `bytes` long in all. */
function noiseWav(path: string, bytes: number) {
  const data = bytes - 44
  const wav = Buffer.alloc(bytes)
  wav.write('RIFF', 0)
  wav.writeUInt32LE(bytes - 8, 4)
  wav.write('WAVEfmt ', 8)
  wav.writeUInt32LE(16, 16)
  wav.writeUInt16LE(1, 20)
  wav.writeUInt16LE(2, 22)
  wav.writeUInt32LE(44_100, 24)
  wav.writeUInt32LE(44_100 * 4, 28)
  wav.writeUInt16LE(4, 32)
  wav.writeUInt16LE(16, 34)
  wav.write('data', 36)
  wav.writeUInt32LE(data, 40)
  let state = 0x1234_5678
  for (let i = 44; i < bytes; i += 2) {
    state = (Math.imul(state, 1_103_515_245) + 12_345) | 0
    wav.writeInt16LE(((state >> 16) & 0x7fff) - 0x4000, i)
  }
  writeFileSync(path, wav)
}

const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)

/** Stands in for the file dialog: the pack goes to `out.faulkpack` in the browser's storage. */
async function chooseOutput(page: Page) {
  await page.evaluate(() => {
    const w = window as unknown as { showSaveFilePicker: unknown; __saved: string[] }
    w.__saved = []
    w.showSaveFilePicker = async (options: { suggestedName: string }) => {
      w.__saved.push(options.suggestedName)
      const root = await navigator.storage.getDirectory()
      const handle = await root.getFileHandle('out.faulkpack', { create: true })
      const real = await handle.createWritable()
      return {
        createWritable: async () => ({
          write: (chunk: Uint8Array) => real.write(chunk as Uint8Array<ArrayBuffer>),
          close: () => real.close(),
          abort: () => real.abort(),
        }),
      }
    }
  })
}

/** The size of the file in the browser's storage and the uncompressed size of the audio blob, from the zip's central directory. */
const zipSizes = (page: Page) =>
  page.evaluate(async () => {
    const root = await navigator.storage.getDirectory()
    const file = await (await root.getFileHandle('out.faulkpack')).getFile()
    const tail = new DataView(await file.slice(Math.max(0, file.size - 65_557)).arrayBuffer())
    let eocd = tail.byteLength - 22
    while (eocd >= 0 && tail.getUint32(eocd, true) !== 0x06054b50) eocd--
    const count = tail.getUint16(eocd + 10, true)
    const size = tail.getUint32(eocd + 12, true)
    const offset = tail.getUint32(eocd + 16, true)
    const dir = new DataView(await file.slice(offset, offset + size).arrayBuffer())
    let blob = -1
    let at = 0
    for (let i = 0; i < count; i++) {
      const nameLength = dir.getUint16(at + 28, true)
      const extra = dir.getUint16(at + 30, true)
      const comment = dir.getUint16(at + 32, true)
      const name = new TextDecoder().decode(new Uint8Array(dir.buffer, at + 46, nameLength))
      if (name.startsWith('blobs/')) blob = dir.getUint32(at + 24, true)
      at += 46 + nameLength + extra + comment
    }
    return { fileSize: file.size, blob }
  })

async function projectWithAudio(page: Page, wav: string) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page)
  test.skip((await hooks(page)) !== 'function', 'the command hooks exist in the dev server only')
  await page.evaluate(() => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun({ type: 'track.create', kind: 'audio' }))
  const track = await page.evaluate(() => {
    const view = (window as unknown as { __faulkView(): { project: { tracks: Record<string, { kind: string }> } } }).__faulkView()
    return Object.entries(view.project.tracks).find(([, t]) => t.kind === 'audio')![0]
  })
  await page.evaluate((id) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun({ type: 'view.select_track', track: id }), track)
  await page.getByTestId('panel-clips').click()
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('audio-import-button').click()
  await (await chooser).setFiles(wav)
  await expect(page.getByTestId('pool-clip')).toHaveCount(1, { timeout: 300_000 })
}

test('the main thread has no long task while a .faulkpack is exported @full', async ({ page }, testInfo) => {
  const { errors } = watch(page)
  const wav = testInfo.outputPath('noise.wav')
  noiseWav(wav, WAV_BYTES)
  await projectWithAudio(page, wav)
  await chooseOutput(page)
  await page.evaluate(() => {
    const w = window as unknown as { __long: number[]; __gaps: number[] }
    w.__long = []
    w.__gaps = []
    new PerformanceObserver((list) => list.getEntries().forEach((e) => w.__long.push(e.duration))).observe({ type: 'longtask', buffered: false })
    let last = performance.now()
    setInterval(() => {
      const now = performance.now()
      w.__gaps.push(now - last)
      last = now
    }, 5)
  })
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-export-pack').click()
  await expect.poll(() => page.evaluate(() => (window as unknown as { __saved: string[] }).__saved.length)).toBe(1)
  await expect(page.getByTestId('export-banner')).toHaveCount(0, { timeout: 600_000 })
  const { long, gaps } = await page.evaluate(() => {
    const w = window as unknown as { __long: number[]; __gaps: number[] }
    return { long: w.__long, gaps: w.__gaps }
  })
  const { fileSize, blob } = await zipSizes(page)
  expect(blob).toBe(WAV_BYTES)
  const sorted = [...gaps].sort((a, b) => a - b)
  const report =
    `${WAV_BYTES >> 20} MiB, zip ${(fileSize / (1 << 20)).toFixed(1)} MiB: ${long.length} long tasks (max ${Math.max(0, ...long).toFixed(0)} ms, ` +
    `total ${long.reduce((a, b) => a + b, 0).toFixed(0)} ms); 5 ms timer: ${gaps.length} ticks, p99 gap ${sorted[Math.floor(sorted.length * 0.99)]?.toFixed(0)} ms, max gap ${sorted.at(-1)?.toFixed(0)} ms`
  testInfo.annotations.push({ type: 'main thread', description: report })
  console.log(report)
  expect(gaps.length).toBeGreaterThan(0)
  expect(Math.max(0, ...long)).toBeLessThan(50)
  expect(errors).toEqual([])
})
