import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #297: the song exported as a WAV file, in a real browser. The render is the real engine module
// in a real Worker, fed by the real project core; the save dialog of the File System Access API
// cannot be driven headless, so the page's `showSaveFilePicker` hands back a file of the origin
// private file system (a `FileSystemFileHandle` like the user's, with the same writable stream and
// the same seek to write the header last). A test reads the file back and checks its header, its
// length and that it is not silent.

const ROW = 16
const RULER = 28

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
const dialog = (page: Page) => page.getByTestId('export-dialog')

/** `showSaveFilePicker` gives a file of the OPFS named as the page suggests; `__pickHold` can make the dialog wait. */
async function simulateSavePicker(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as {
      __picks: { suggestedName: string; startIn: unknown }[]
      __pickHold?: Promise<void>
      showSaveFilePicker: (options: { suggestedName: string; startIn: unknown }) => Promise<FileSystemFileHandle>
    }
    w.__picks = []
    w.showSaveFilePicker = async (options) => {
      w.__picks.push({ suggestedName: options.suggestedName, startIn: options.startIn })
      await w.__pickHold
      return (await navigator.storage.getDirectory()).getFileHandle(options.suggestedName, { create: true })
    }
  })
}

/** A browser without the save dialog: the export is a download. */
async function withoutSavePicker(page: Page) {
  await page.addInitScript(() => Object.defineProperty(window, 'showSaveFilePicker', { value: undefined, configurable: true }))
}

interface Wav {
  size: number
  riff: string
  format: number
  channels: number
  rate: number
  bits: number
  data: number
  frames: number
  peak: number
  lastPeak: number
  firstPeak: number
}

/** Reads a WAV file of the OPFS and summarizes it: its header and its signal (24 bits, or 16). */
const readWav = (page: Page, name: string): Promise<Wav | null> =>
  page.evaluate(async (file) => {
    let blob: File
    try {
      blob = await (await (await navigator.storage.getDirectory()).getFileHandle(file)).getFile()
    } catch {
      return null
    }
    return summarize(new DataView(await blob.arrayBuffer()))

    function summarize(view: DataView) {
      const text = (at: number, n: number) => String.fromCharCode(...new Uint8Array(view.buffer, at, n))
      const bits = view.getUint16(34, true)
      const bytes = bits / 8
      const frames = (view.byteLength - 44) / (2 * bytes)
      const sample = (frame: number, channel: number) => {
        const at = 44 + (frame * 2 + channel) * bytes
        if (bits === 16) return view.getInt16(at, true) / 32768
        return (view.getInt32(at - 1, true) >> 8) / 8388608
      }
      const peakOf = (from: number, to: number) => {
        let peak = 0
        for (let f = Math.max(0, from); f < Math.min(frames, to); f++) peak = Math.max(peak, Math.abs(sample(f, 0)), Math.abs(sample(f, 1)))
        return peak
      }
      return {
        size: view.byteLength,
        riff: text(0, 4) + text(8, 8) + text(36, 4),
        format: view.getUint16(20, true),
        channels: view.getUint16(22, true),
        rate: view.getUint32(24, true),
        bits,
        data: view.getUint32(40, true),
        frames,
        peak: peakOf(0, frames),
        lastPeak: peakOf(frames - 64, frames),
        firstPeak: peakOf(0, 4800),
      }
    }
  }, name)

/**
 * A clip in the piano roll with four notes in its first bar, drawn with the pointer: the same in
 * every project of the run. `bars` lengthens the clip afterwards, so the song lasts that long (the
 * notes stay in bar 1: the other bars are silent).
 */
async function makeSong(page: Page, bars = 1) {
  // Tall enough for the roll to show the whole range of the notes below under the ruler, even with the toolbar on two lines.
  await page.setViewportSize({ width: 1280, height: 1000 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '0')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await page.getByTestId('roll-snap').selectOption('beat')
  const stage = page.getByTestId('roll-stage')
  const box = (await stage.boundingBox())!
  const ppt = Number(await page.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollY = Number(await stage.getAttribute('data-scroll-y'))
  for (const [tick, pitch] of [
    [20, 57],
    [980, 60],
    [1940, 64],
    [2900, 69],
  ]) {
    await page.mouse.click(box.x + tick * ppt, box.y + RULER + (127 - pitch) * ROW - scrollY + ROW / 2)
  }
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '4')
  if (bars > 1) {
    await page.getByTestId('roll-length').fill(String(bars))
    await page.getByTestId('roll-length').press('Enter')
    await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-clip-length', String(bars * 3840))
  }
}

async function openDialog(page: Page) {
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-export').click()
  await expect(dialog(page)).toBeVisible()
}

test('exports the song as a 24-bit WAV file while the song plays: header, length, not silent, ends quiet', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await simulateSavePicker(page)
  await makeSong(page)
  await page.getByTestId('project-name').waitFor()

  // The song plays live; the export must not disturb it, nor the other way round.
  await page.getByTestId('transport-play').click()
  const tick = async () => Number(await page.getByTestId('feedback').getAttribute('data-tick'))
  await expect.poll(tick).toBeGreaterThan(0)
  const before = await tick()

  await openDialog(page)
  await expect(page.getByTestId('export-pcm24')).toBeChecked()
  await expect(page.getByTestId('export-rate-48000')).toBeChecked()
  await expect(page.getByTestId('export-song')).toBeChecked()
  // No loop region yet: the loop is not offered.
  await expect(page.getByTestId('export-loop')).toBeDisabled()
  await page.getByTestId('export-start').click()
  await expect(dialog(page)).toHaveAttribute('data-phase', 'done', { timeout: 60_000 })
  await expect(page.getByTestId('export-status')).toContainText('Saved')
  await expect(page.getByTestId('export-status')).toContainText('.wav')
  await expect(page.getByTestId('export-clipped')).toHaveCount(0)

  expect(await page.evaluate(() => (window as unknown as { __picks: unknown[] }).__picks)).toMatchObject([{ suggestedName: expect.stringMatching(/\.wav$/) }])
  const name = await page.evaluate(() => (window as unknown as { __picks: { suggestedName: string }[] }).__picks[0].suggestedName)
  const wav = (await readWav(page, name))!
  expect(wav.riff).toBe('RIFFWAVEfmt data')
  expect(wav.format).toBe(1)
  expect(wav.channels).toBe(2)
  expect(wav.rate).toBe(48_000)
  expect(wav.bits).toBe(24)
  expect(wav.data).toBe(wav.size - 44)
  // One bar at 120 BPM is two seconds; the synth's release (and nothing else) follows.
  expect(wav.frames).toBeGreaterThanOrEqual(2 * 48_000)
  expect(wav.frames).toBeLessThan(3.5 * 48_000)
  // The first note is at the very start, and the file is loud enough to hear.
  expect(wav.firstPeak).toBeGreaterThan(0.01)
  expect(wav.peak).toBeGreaterThan(0.05)
  expect(wav.peak).toBeLessThanOrEqual(1)
  // It ends where the sound ends, under -60 dBFS.
  expect(wav.lastPeak).toBeLessThan(0.001)

  // The live song kept playing all along.
  expect(await tick()).toBeGreaterThan(before)
  await dialog(page).getByTestId('export-close').click()
  await expect(dialog(page)).toHaveCount(0)
  expect(seen.errors).toEqual([])
})

test('four bars: the file lasts eight seconds and a bit, and a range of bars is that range', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await simulateSavePicker(page)
  await makeSong(page, 4)
  const exportAs = async (nth: number) => {
    await page.getByTestId('export-start').click()
    await expect(dialog(page)).toHaveAttribute('data-phase', 'done', { timeout: 60_000 })
    const name = await page.evaluate((n) => (window as unknown as { __picks: { suggestedName: string }[] }).__picks[n].suggestedName, nth)
    return (await readWav(page, name))!
  }

  // The song: four bars are eight seconds at 120 BPM, and the release of the last note follows.
  await openDialog(page)
  const song = await exportAs(0)
  expect(song.frames).toBeGreaterThanOrEqual(8 * 48_000)
  expect(song.frames).toBeLessThan(9.5 * 48_000)
  expect(song.firstPeak).toBeGreaterThan(0.01)
  expect(song.peak).toBeGreaterThan(0.05)
  await page.getByTestId('export-close').click()

  // Bars 2 to 3: four seconds of what is there, which is nothing (the notes are in bar 1): the file
  // is the range and no more, and it says it is silent.
  await openDialog(page)
  await page.getByTestId('export-bars').check()
  await expect(page.getByTestId('export-from-bar')).toHaveValue('1')
  await expect(page.getByTestId('export-to-bar')).toHaveValue('4')
  await page.getByTestId('export-from-bar').fill('2')
  await page.getByTestId('export-to-bar').fill('3')
  const middle = await exportAs(1)
  expect(middle.frames).toBe(4 * 48_000)
  expect(middle.peak).toBe(0)
  await expect(page.getByTestId('export-peak')).toContainText('silent')
  await page.getByTestId('export-close').click()

  // Bar 1 alone: two seconds and the release of its notes, the first one at the start of the file.
  await openDialog(page)
  await page.getByTestId('export-bars').check()
  await page.getByTestId('export-from-bar').fill('1')
  await page.getByTestId('export-to-bar').fill('1')
  const first = await exportAs(2)
  expect(first.frames).toBeGreaterThanOrEqual(2 * 48_000)
  expect(first.frames).toBeLessThan(3.5 * 48_000)
  expect(first.firstPeak).toBeGreaterThan(0.01)
  await page.getByTestId('export-close').click()

  // Bars that make no range cannot be exported; bars past the end of the song are refused by the
  // project, and the dialog says so.
  await openDialog(page)
  await page.getByTestId('export-bars').check()
  await page.getByTestId('export-from-bar').fill('3')
  await page.getByTestId('export-to-bar').fill('2')
  await expect(page.getByTestId('export-bad-bars')).toBeVisible()
  await expect(page.getByTestId('export-start')).toBeDisabled()
  await page.getByTestId('export-from-bar').fill('5')
  await page.getByTestId('export-to-bar').fill('6')
  await page.getByTestId('export-start').click()
  await expect(page.getByTestId('export-error')).toBeVisible()
  expect(await page.evaluate(() => (window as unknown as { __picks: unknown[] }).__picks.length)).toBe(3)
  // Not a page bug, but the project refusing a command is logged as a warning: that is the only one.
  expect(seen.errors).toEqual([])
})

test('the dialog offers the 16-bit format and the rates, and the file follows', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await simulateSavePicker(page)
  await makeSong(page)
  await openDialog(page)
  await page.getByTestId('export-pcm16').check()
  await page.getByTestId('export-rate-44100').check()
  await page.getByTestId('export-start').click()
  await expect(dialog(page)).toHaveAttribute('data-phase', 'done', { timeout: 60_000 })
  const name = await page.evaluate(() => (window as unknown as { __picks: { suggestedName: string }[] }).__picks[0].suggestedName)
  const wav = (await readWav(page, name))!
  expect(wav).toMatchObject({ bits: 16, rate: 44_100, channels: 2, format: 1 })
  expect(wav.frames).toBeGreaterThanOrEqual(2 * 44_100)
  expect(wav.peak).toBeGreaterThan(0.05)
  expect(seen.errors).toEqual([])
})

test('leaving the file dialog or cancelling while it is open renders nothing and keeps no file', async ({ page }) => {
  const seen = watch(page)
  await simulateSavePicker(page)
  await page.addInitScript(() => {
    const w = window as unknown as { __pickHold?: Promise<void>; __release: () => void }
    w.__pickHold = new Promise<void>((resolve) => (w.__release = resolve))
  })
  await makeSong(page)
  await openDialog(page)
  await page.getByTestId('export-start').click()
  await expect(dialog(page)).toHaveAttribute('data-phase', 'picking')
  await page.getByTestId('export-abort').click()
  await expect(dialog(page)).toHaveAttribute('data-phase', 'options')
  // The dialog of the browser answers after the cancel: its file is dropped, nothing renders.
  await page.evaluate(() => (window as unknown as { __release: () => void }).__release())
  await expect(dialog(page)).toHaveAttribute('data-phase', 'options')
  await page.waitForTimeout(300)
  await expect(dialog(page)).toHaveAttribute('data-phase', 'options')
  expect(seen.errors).toEqual([])
})

test('a browser without the save dialog downloads the file', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await withoutSavePicker(page)
  await makeSong(page)
  await openDialog(page)
  await expect(page.getByTestId('export-download-note')).toBeVisible()
  const download = page.waitForEvent('download')
  await page.getByTestId('export-start').click()
  const file = await download
  expect(file.suggestedFilename()).toMatch(/\.wav$/)
  await expect(dialog(page)).toHaveAttribute('data-phase', 'done', { timeout: 60_000 })
  await expect(page.getByTestId('export-status')).toContainText('Downloaded')
  const path = await file.path()
  const { readFileSync } = await import('node:fs')
  const bytes = readFileSync(path)
  expect(bytes.subarray(0, 4).toString()).toBe('RIFF')
  expect(bytes.subarray(8, 12).toString()).toBe('WAVE')
  expect(bytes.readUInt32LE(40)).toBe(bytes.length - 44)
  expect(bytes.length).toBeGreaterThan(2 * 48_000 * 6)
  expect(seen.errors).toEqual([])
})

test('the dialog is in French when the page is', async ({ browser }) => {
  const context = await browser.newContext({ locale: 'fr-FR' })
  const page = await context.newPage()
  const seen = watch(page)
  await simulateSavePicker(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await openDialog(page)
  await expect(dialog(page).getByRole('heading')).toHaveText('Exporter l’audio')
  // Nothing to export yet: the button waits, and says why.
  await expect(page.getByTestId('export-start')).toBeDisabled()
  await expect(page.getByTestId('export-empty')).toContainText('Place d’abord un clip')
  await page.keyboard.press('Escape')
  await expect(dialog(page)).toHaveCount(0)
  expect(seen.errors).toEqual([])
  await context.close()
})

test.describe('with the dispatcher hook (dev server)', () => {
  test('an agent’s project.export_audio opens the dialog with its settings; the loop region exports once', async ({ page }) => {
    test.setTimeout(120_000)
    const seen = watch(page)
    await simulateSavePicker(page)
    await makeSong(page)
    test.skip(
      (await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function',
      'the dispatcher hook exists in the dev server only',
    )
    // A loop region over the first beat only.
    await page.evaluate(() =>
      (window as unknown as { __faulkRun: (c: unknown) => void }).__faulkRun({ type: 'transport.set_loop', loop_region: { start: 0, end: 960 } }),
    )
    await page.evaluate(() =>
      (window as unknown as { __faulkRun: (c: unknown) => void }).__faulkRun({
        type: 'project.export_audio',
        region: 'loop',
        bit_depth: 'pcm16',
        sample_rate: 96000,
      }),
    )
    await expect(dialog(page)).toBeVisible()
    await expect(page.getByTestId('export-loop')).toBeChecked()
    await expect(page.getByTestId('export-pcm16')).toBeChecked()
    await expect(page.getByTestId('export-rate-96000')).toBeChecked()
    await page.getByTestId('export-start').click()
    await expect(dialog(page)).toHaveAttribute('data-phase', 'done', { timeout: 60_000 })
    const name = await page.evaluate(() => (window as unknown as { __picks: { suggestedName: string }[] }).__picks[0].suggestedName)
    const wav = (await readWav(page, name))!
    // A beat is half a second at 120 BPM: 48000 frames at 96 kHz, and the release after it.
    expect(wav).toMatchObject({ bits: 16, rate: 96_000 })
    expect(wav.frames).toBeGreaterThanOrEqual(48_000)
    expect(wav.frames).toBeLessThan(2 * 96_000)
    expect(wav.peak).toBeGreaterThan(0.05)
    expect(seen.errors).toEqual([])
  })

  test('a three-minute song renders much faster than it plays', async ({ page }) => {
    test.setTimeout(300_000)
    const seen = watch(page)
    await simulateSavePicker(page)
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.goto('/app')
    await startAudio(page)
    await expect(page.getByTestId('engine-mode')).toBeVisible()
    test.skip(
      (await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function',
      'the dispatcher hook exists in the dev server only',
    )
    // 90 bars at 120 BPM: three minutes of a bass line and an arpeggio, 1 800 notes.
    await page.evaluate(() => {
      const w = window as unknown as {
        __faulkRun: (c: unknown) => void
        __faulkView: () => { project: { tracks: Record<string, { clips: Record<string, unknown> }> } }
      }
      const track = Object.keys(w.__faulkView().project.tracks)[0]
      w.__faulkRun({ type: 'clip.create', track, at: { at: 'arrangement', start: 0 }, length: 90 * 3840 })
      const clip = Object.keys(w.__faulkView().project.tracks[track].clips)[0]
      const notes = []
      for (let bar = 0; bar < 90; bar++) {
        for (let beat = 0; beat < 4; beat++) {
          notes.push({ start: bar * 3840 + beat * 960, pitch: 36 + (bar % 4) * 2, length: 720, velocity: 100, cents: 0, chance: 100, repeat: 1 })
          for (let k = 0; k < 4; k++)
            notes.push({ start: bar * 3840 + beat * 960 + k * 240, pitch: 60 + k * 4, length: 200, velocity: 80, cents: 0, chance: 100, repeat: 1 })
        }
      }
      w.__faulkRun({ type: 'clip.notes.add', clip, notes })
    })
    await openDialog(page)
    await page.getByTestId('export-start').click()
    await expect(dialog(page)).toHaveAttribute('data-phase', 'done', { timeout: 240_000 })
    const ms = Number(await dialog(page).getAttribute('data-render-ms'))
    const name = await page.evaluate(() => (window as unknown as { __picks: { suggestedName: string }[] }).__picks[0].suggestedName)
    const wav = (await readWav(page, name))!
    console.log(
      `export: ${(wav.frames / wav.rate).toFixed(1)} s of audio, ${(wav.size / 1e6).toFixed(1)} MB, rendered and written in ${(ms / 1000).toFixed(2)} s`,
    )
    expect(wav.frames).toBeGreaterThanOrEqual(180 * 48_000)
    expect(wav.peak).toBeGreaterThan(0.05)
    // Much faster than real time (180 s): a gross regression only, the figure is printed.
    expect(ms).toBeLessThan(90_000)
    expect(seen.errors).toEqual([])
  })
})
