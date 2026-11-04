import { writeFileSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #617: the `.faulkpack` export is written in a stream. A project with a large audio file (generated here, a WAV
// of about 90 MiB, never committed) is exported to a file the user chooses (the dialog is stood in for by a file
// of the browser's own storage, since a native dialog cannot be driven): the media are read by pieces, the zip is
// complete and holds the audio whole, the JavaScript heap stays far under the size of the file, and an export
// can be cancelled while it runs, leaving no file.

test.describe.configure({ timeout: 240_000 })

const WAV_BYTES = 90 << 20

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

/** Stands in for the file dialog: the pack goes to `out.faulkpack` in the browser's storage, `delay` ms per piece. */
async function chooseOutput(page: Page, delay: number) {
  await page.evaluate((pause) => {
    const w = window as unknown as { showSaveFilePicker: unknown; __peakHeap: number; __saved: string[] }
    w.__saved = []
    w.showSaveFilePicker = async (options: { suggestedName: string }) => {
      w.__saved.push(options.suggestedName)
      const root = await navigator.storage.getDirectory()
      const handle = await root.getFileHandle('out.faulkpack', { create: true })
      const real = await handle.createWritable()
      return {
        createWritable: async () => ({
          write: async (chunk: Uint8Array) => {
            if (pause) await new Promise((resolve) => setTimeout(resolve, pause))
            await real.write(chunk as Uint8Array<ArrayBuffer>)
          },
          close: () => real.close(),
          abort: () => real.abort(),
        }),
      }
    }
    // The heap the page itself holds (the blobs go through the Worker and the core's memory in pieces).
    w.__peakHeap = 0
    setInterval(() => {
      const used = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? 0
      w.__peakHeap = Math.max(w.__peakHeap, used)
    }, 20)
  }, delay)
}

/** The entries of the zip in the browser's storage: name and uncompressed size, from its central directory. */
const zipEntries = (page: Page) =>
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
    const entries: { name: string; size: number }[] = []
    let at = 0
    for (let i = 0; i < count; i++) {
      const nameLength = dir.getUint16(at + 28, true)
      const extra = dir.getUint16(at + 30, true)
      const comment = dir.getUint16(at + 32, true)
      const name = new TextDecoder().decode(new Uint8Array(dir.buffer, at + 46, nameLength))
      entries.push({ name, size: dir.getUint32(at + 24, true) })
      at += 46 + nameLength + extra + comment
    }
    return { fileSize: file.size, entries }
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
  await expect(page.getByTestId('pool-clip')).toHaveCount(1, { timeout: 120_000 })
}

test('a project with 90 MiB of audio is exported to a chosen file in a stream, and an export can be cancelled @full', async ({ page }, testInfo) => {
  const { errors } = watch(page)
  const wav = testInfo.outputPath('noise.wav')
  noiseWav(wav, WAV_BYTES)
  await projectWithAudio(page, wav)
  await chooseOutput(page, 0)

  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-export-pack').click()
  await expect.poll(() => page.evaluate(() => (window as unknown as { __saved: string[] }).__saved.length)).toBe(1)
  await expect(page.getByTestId('export-banner')).toHaveCount(0, { timeout: 120_000 })
  const { fileSize, entries } = await zipEntries(page)
  expect(fileSize).toBeGreaterThan(WAV_BYTES / 2)
  const audio = entries.find((e) => e.name.startsWith('blobs/'))
  expect(audio?.size).toBe(WAV_BYTES)
  expect(entries.map((e) => e.name)).toContain('project.json')
  const peak = await page.evaluate(() => (window as unknown as { __peakHeap: number }).__peakHeap)
  testInfo.annotations.push({
    type: 'peak JS heap',
    description: `${(peak / (1 << 20)).toFixed(1)} MiB for a ${WAV_BYTES >> 20} MiB audio file, zip ${(fileSize / (1 << 20)).toFixed(1)} MiB`,
  })
  console.log(`peak JS heap ${(peak / (1 << 20)).toFixed(1)} MiB; zip ${(fileSize / (1 << 20)).toFixed(1)} MiB`)
  // `performance.memory` is Chromium's; without it nothing was measured, which must not pass.
  expect(peak).toBeGreaterThan(0)
  expect(peak).toBeLessThan(WAV_BYTES * 0.8)

  // A slow disk: the banner shows, Cancel stops the export, and no file is left.
  await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory()
    await root.removeEntry('out.faulkpack')
  })
  await chooseOutput(page, 250)
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-export-pack').click()
  await expect(page.getByTestId('export-banner')).toBeVisible({ timeout: 30_000 })
  await page.getByTestId('export-cancel').click()
  await expect(page.getByTestId('storage-notice')).toContainText('Export cancelled', { timeout: 30_000 })
  const left = await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory()
    const names: string[] = []
    for await (const entry of (root as unknown as { values(): AsyncIterable<FileSystemHandle> }).values()) names.push(entry.name)
    return names.filter((n) => n.startsWith('out.faulkpack'))
  })
  expect(left.every((n) => n === 'out.faulkpack')).toBe(true)
  const size = await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory()
    return (await (await root.getFileHandle('out.faulkpack')).getFile()).size
  })
  expect(size).toBe(0)
  expect(errors).toEqual([])
})
