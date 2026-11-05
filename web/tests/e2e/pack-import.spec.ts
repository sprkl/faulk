import { readFileSync, writeFileSync } from 'node:fs'
import { expect, test, type Page, type TestInfo } from '@playwright/test'
import { engineUp, watch } from './support'

// #682: a `.faulkpack` is imported in pieces by the store's Worker, whatever its size. A project with a large audio
// file (generated here, `FAULK_PACK_MIB` MiB of noise, 300 by default, never committed) is exported as a download,
// the project is removed from the browser, then the pack is chosen for an import. A run cancelled midway and a
// damaged copy change nothing; the whole one opens and plays. During it, the memory of the page (JS heap and buffers), the memory
// of the Worker's instance of the project module, the Long Tasks and the gaps of a 5 ms timer are recorded.

test.describe.configure({ timeout: 1_800_000 })

const WAV_BYTES = (Number(process.env.FAULK_PACK_MIB) || 300) << 20

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

type Hooks = { __faulkRun(c: object): boolean; __faulkView(): { project: { id: string; tracks: Record<string, { kind: string }> } } }
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const projectId = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__faulkView().project.id)
const audioTracks = (page: Page) =>
  page.evaluate(() => Object.values((window as unknown as Hooks).__faulkView().project.tracks).filter((t) => t.kind === 'audio').length)

async function startAudio(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page)
}

/** A project with the audio file, exported as a download saved to `out` (the browser keeps no save dialog here); its identity. */
async function exportedProject(page: Page, wav: string, out: string): Promise<string> {
  await startAudio(page)
  test.skip((await hooks(page)) !== 'function', 'the command hooks exist in the dev server only')
  await page.evaluate(() => (window as unknown as Hooks).__faulkRun({ type: 'track.create', kind: 'audio' }))
  const track = await page.evaluate(() => Object.entries((window as unknown as Hooks).__faulkView().project.tracks).find(([, t]) => t.kind === 'audio')![0])
  await page.evaluate((id) => (window as unknown as Hooks).__faulkRun({ type: 'view.select_track', track: id }), track)
  await page.getByTestId('panel-clips').click()
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('audio-import-button').click()
  await (await chooser).setFiles(wav)
  await expect(page.getByTestId('pool-clip')).toHaveCount(1, { timeout: 600_000 })
  await page.evaluate(() => void ((window as unknown as { showSaveFilePicker?: unknown }).showSaveFilePicker = undefined))
  const download = page.waitForEvent('download', { timeout: 900_000 })
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-export-pack').click()
  await (await download).saveAs(out)
  return projectId(page)
}

/** A copy of the pack `from` with one byte of the middle (the audio) changed. */
function damagedCopy(from: string, to: string) {
  const bytes = readFileSync(from)
  bytes[Math.floor(bytes.length / 2)] ^= 0x55
  writeFileSync(to, bytes)
}

/** The browser keeps no project any more (the pack's project must come in as a new one); the packs stay. */
async function forgetProjects(page: Page) {
  await page.goto('/')
  await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory()
    await (await root.getDirectoryHandle('faulk')).removeEntry('browser', { recursive: true }).catch(() => undefined)
  })
}

/** Imports the pack at `path` through the project menu and the file chooser. */
async function importPack(page: Page, path: string) {
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-import-pack').click()
  await (await chooser).setFiles(path)
}

/** Records, from now on, the Long Tasks and the gaps of a 5 ms timer of the page. */
const record = (page: Page) =>
  page.evaluate(() => {
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

const recorded = (page: Page) =>
  page.evaluate(() => {
    const w = window as unknown as { __long: number[]; __gaps: number[] }
    return { long: w.__long, gaps: w.__gaps }
  })

/**
 * Samples the page's memory every 100 ms through the DevTools protocol: its JS heap plus the backing stores of its
 * ArrayBuffers (where a pack read whole lives), which `performance.memory` does not count. `stop` gives the first
 * sample and the peak, in bytes.
 */
async function sampleMemory(page: Page) {
  const cdp = await page.context().newCDPSession(page)
  const read = async () => {
    const usage = (await cdp.send('Runtime.getHeapUsage')) as { usedSize: number; backingStorageSize?: number }
    return usage.usedSize + (usage.backingStorageSize ?? 0)
  }
  const base = await read()
  let peak = base
  let busy = false
  const timer = setInterval(() => {
    if (busy) return
    busy = true
    read()
      .then((now) => (peak = Math.max(peak, now)))
      .catch(() => undefined)
      .finally(() => (busy = false))
  }, 100)
  return {
    stop: async () => {
      clearInterval(timer)
      peak = Math.max(peak, await read().catch(() => 0))
      await cdp.detach().catch(() => undefined)
      return { base, peak }
    },
  }
}

/** The most memory the store Worker's instances of the project module took for an import (`pack-worker.ts`), in bytes. */
async function workerMemory(page: Page): Promise<number | null> {
  for (const worker of page.workers()) {
    const value = await worker.evaluate(() => (globalThis as { __faulkUnpackMemory?: number }).__faulkUnpackMemory ?? null).catch(() => null)
    if (value !== null) return value
  }
  return null
}

test('a large .faulkpack is imported in pieces: cancel, a damaged pack, then the whole one opens and plays @full', async ({ page }, testInfo) => {
  const { errors } = watch(page)
  const wav = testInfo.outputPath('noise.wav')
  noiseWav(wav, WAV_BYTES)
  const pack = testInfo.outputPath('out.faulkpack')
  const bad = testInfo.outputPath('bad.faulkpack')
  const packed = await exportedProject(page, wav, pack)
  damagedCopy(pack, bad)
  await forgetProjects(page)
  await startAudio(page)
  const before = await projectId(page)
  expect(before).not.toBe(packed)
  // `FAULK_PACK_MEASURE_ONLY=1`: the measured import alone (to measure a page that predates the import in pieces).
  if (!process.env.FAULK_PACK_MEASURE_ONLY) await cancelAndDamaged(page, before, pack, bad)

  // The whole pack, measured.
  await record(page)
  const memory = await sampleMemory(page)
  const started = Date.now()
  await importPack(page, pack)
  await expect.poll(() => projectId(page), { timeout: 1_200_000 }).toBe(packed)
  const { base, peak } = await memory.stop()
  await finish(page, testInfo, started, errors, base, peak)
})

/** Cancelled midway (the banner says how far it is), then a damaged copy (refused with its reason): the project stays, nothing is kept. */
async function cancelAndDamaged(page: Page, before: string, pack: string, bad: string) {
  await importPack(page, pack)
  await expect(page.getByTestId('import-banner')).toBeVisible({ timeout: 60_000 })
  await expect.poll(async () => (await page.getByTestId('import-banner').textContent()) ?? '', { timeout: 120_000 }).toMatch(/[1-9]\d?%/)
  await page.getByTestId('import-cancel').click()
  await expect(page.getByTestId('storage-notice')).toContainText('Import cancelled', { timeout: 60_000 })
  expect(await projectId(page)).toBe(before)
  await page.getByTestId('storage-notice').getByRole('button').last().click()

  // A damaged pack: refused with its reason, nothing changes.
  await importPack(page, bad)
  await expect(page.getByTestId('storage-notice')).toContainText('not a valid .faulkpack', { timeout: 600_000 })
  expect(await projectId(page)).toBe(before)
  const kept = await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory()
    const browser = await (await root.getDirectoryHandle('faulk')).getDirectoryHandle('browser')
    const names: string[] = []
    for await (const name of (browser as unknown as { keys(): AsyncIterable<string> }).keys()) names.push(name)
    return names.length
  })
  expect(kept).toBeLessThanOrEqual(1)
  await page.getByTestId('storage-notice').getByRole('button').last().click()
}

async function finish(page: Page, testInfo: TestInfo, started: number, errors: string[], base: number, heap: number) {
  const importMs = Date.now() - started
  const { long, gaps } = await recorded(page)
  const wasm = await workerMemory(page)
  expect(await audioTracks(page)).toBe(1)
  const sorted = [...gaps].sort((a, b) => a - b)
  const report =
    `${WAV_BYTES >> 20} MiB of audio, import ${(importMs / 1000).toFixed(1)} s: page memory (JS heap and buffers) peak ${(heap / (1 << 20)).toFixed(1)} MiB, ${((heap - base) / (1 << 20)).toFixed(1)} MiB over the start, ` +
    `Worker module memory ${wasm === null ? 'n/a' : (wasm / (1 << 20)).toFixed(1) + ' MiB'}; ${long.length} long tasks (max ${Math.max(0, ...long).toFixed(0)} ms); ` +
    `5 ms timer: ${gaps.length} ticks, p99 gap ${sorted[Math.floor(sorted.length * 0.99)]?.toFixed(0)} ms, max gap ${sorted.at(-1)?.toFixed(0)} ms`
  testInfo.annotations.push({ type: 'import', description: report })
  console.log(report)

  // Then it plays.
  await page.evaluate(() => (window as unknown as Hooks).__faulkRun({ type: 'transport.play' }))
  await page.waitForTimeout(1500)
  await page.evaluate(() => (window as unknown as Hooks).__faulkRun({ type: 'transport.stop' }))
  // The page's memory does not follow the size of the pack.
  expect(heap - base).toBeLessThan(64 << 20)
  if (wasm !== null) expect(wasm).toBeLessThan(64 << 20)
  expect(errors).toEqual([])
}
