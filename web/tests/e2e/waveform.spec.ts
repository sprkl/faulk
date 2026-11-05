import { expect, test, type Page } from '@playwright/test'
import { BOOT, watch } from './support'

// AU-2 (#111): the waveform drawn from its peaks, on its test surface (`/dev/waveform`, served by the dev
// server and by the e2e build, which sets VITE_FAULK_DEV_ROUTES): one hour of stereo at 48 kHz. The
// canvas itself (pixels read back), the level each zoom reads, and the cost of scrolling across the hour
// while zooming from the whole hour down past the finest level.

const STRICT = process.env.FAULK_E2E_STRICT_TIMING === '1'
if (process.env.FAULK_E2E_STRICT_TIMING !== undefined && !['0', '1'].includes(process.env.FAULK_E2E_STRICT_TIMING)) {
  throw new Error('FAULK_E2E_STRICT_TIMING must be 1 or 0')
}

const surface = (page: Page) => page.getByTestId('waveform-surface')
const waveform = (page: Page) => page.getByTestId('waveform')

async function open(page: Page) {
  const seen = watch(page)
  await page.goto('/dev/waveform')
  await expect(surface(page)).toBeVisible(BOOT)
  await expect.poll(async () => Number(await waveform(page).getAttribute('data-draw-count')), BOOT).toBeGreaterThan(0)
  return seen
}

/** The alpha of a canvas pixel, in device pixels. */
async function alpha(page: Page, x: number, y: number): Promise<number> {
  return page.evaluate(
    ([px, py]) => {
      const canvas = document.querySelector('[data-testid=waveform] canvas') as HTMLCanvasElement
      return canvas.getContext('2d')!.getImageData(px!, py!, 1, 1).data[3]!
    },
    [x, y],
  )
}

/** The colour of the most opaque pixel of the canvas's middle column, in device pixels. */
async function ink(page: Page): Promise<[number, number, number]> {
  return page.evaluate(() => {
    const canvas = document.querySelector('[data-testid=waveform] canvas') as HTMLCanvasElement
    const data = canvas.getContext('2d')!.getImageData(Math.floor(canvas.width / 2), 0, 1, canvas.height).data
    let best = 0
    for (let i = 0; i < data.length; i += 4) if (data[i + 3]! > data[best + 3]!) best = i
    return [data[best]!, data[best + 1]!, data[best + 2]!] as [number, number, number]
  })
}

const near = (a: readonly number[], b: readonly number[]) => a.every((v, i) => Math.abs(v - b[i]!) <= 2)

// #687: the surface draws a track's hue (track-08, #4ac495): as it is in the dark theme, with 40 % of the light
// theme's ink (#131211) in the light one, and again at once when the theme changes.
test('takes the theme: the hue in dark, the hue with ink in light, and redraws when the theme changes', async ({ page }) => {
  const hue = [0x4a, 0xc4, 0x95]
  const light = hue.map((v, i) => Math.round(v * 0.6 + [0x13, 0x12, 0x11][i]! * 0.4))
  await page.addInitScript(() => localStorage.setItem('faulk.theme', 'system'))
  await page.emulateMedia({ colorScheme: 'dark' })
  const seen = await open(page)
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  expect(near(await ink(page), hue), String(await ink(page))).toBe(true)
  await page.emulateMedia({ colorScheme: 'light' })
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light')
  await expect.poll(async () => near(await ink(page), light)).toBe(true)
  await page.emulateMedia({ colorScheme: 'dark' })
  await expect.poll(async () => near(await ink(page), hue)).toBe(true)
  expect(seen.errors).toEqual([])
})

test.describe('the waveform of an hour of audio', () => {
  test.describe.configure({ timeout: 120_000 })

  for (const theme of ['dark', 'light'] as const)
    test(`draws its two channels from the peaks, around the middle of each lane and not beyond its amplitude (${theme})`, async ({ page }) => {
      await page.addInitScript((t) => localStorage.setItem('faulk.theme', t), theme)
      const seen = await open(page)
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme)
      const box = (await waveform(page).boundingBox())!
      const dpr = await page.evaluate(() => window.devicePixelRatio || 1)
      const x = Math.round((box.width / 2) * dpr)
      // Two lanes: the middle of each is drawn, the top edge of the canvas (above any peak but a beat's) is not.
      const lane = box.height / 2
      expect(await alpha(page, x, Math.round((lane / 2) * dpr))).toBeGreaterThan(0)
      expect(await alpha(page, x, Math.round((lane + lane / 2) * dpr))).toBeGreaterThan(0)
      expect(await alpha(page, x, 0)).toBe(0)
      // 16 384 frames a CSS pixel: the top level.
      expect(await waveform(page).getAttribute('data-level')).toBe(String(dpr >= 2 ? 2 : 3))
      expect(seen.errors).toEqual([])
    })

  test('reads a finer level as it zooms in, a coarser one as it zooms out', async ({ page }) => {
    const seen = await open(page)
    const levels = new Set<string>()
    /** One Ctrl + wheel step on the surface, then the level the next drawn frame read. */
    const zoom = (deltaY: number) =>
      page.evaluate(
        (deltaY) =>
          new Promise<string>((done) => {
            const root = document.querySelector('[data-testid=waveform-surface]') as HTMLElement
            const wave = document.querySelector('[data-testid=waveform]') as HTMLElement
            const box = root.getBoundingClientRect()
            root.dispatchEvent(
              new WheelEvent('wheel', { bubbles: true, cancelable: true, ctrlKey: true, deltaY, clientX: box.left + box.width / 2, clientY: box.top + 10 }),
            )
            // The draw runs in the next animation frame; read after it.
            requestAnimationFrame(() => requestAnimationFrame(() => done(wave.dataset.level ?? '')))
          }),
        deltaY,
      )
    const perPixel = async () => Number(await surface(page).getAttribute('data-frames-per-pixel'))
    // In by 2 per step, from 16 384 frames a pixel down to 1 (level 0 from 256 frames a device pixel down).
    for (let step = 0; step < 14; step++) levels.add(await zoom(-120))
    expect(await perPixel()).toBe(1)
    expect(await waveform(page).getAttribute('data-level')).toBe('0')
    // Out to the whole hour in the width: the top level.
    const width = await surface(page).evaluate((el) => el.clientWidth)
    for (let step = 0; step < 30 && (await perPixel()) < (3600 * 48_000) / width; step++) levels.add(await zoom(120))
    expect(await perPixel()).toBe((3600 * 48_000) / width)
    expect(await waveform(page).getAttribute('data-level')).toBe('3')
    expect([...levels].sort()).toEqual(['0', '1', '2', '3'])
    expect(seen.errors).toEqual([])
  })

  test('scrolls across the hour, zooming in and out, at the pace of the screen', async ({ page }) => {
    const seen = await open(page)
    const result = await page.evaluate(
      () =>
        new Promise<{ frames: number; intervals: number[]; draws: number[]; drawn: number; farthest: number; total: number; levels: string[] }>((done) => {
          const root = document.querySelector('[data-testid=waveform-surface]') as HTMLElement
          const wave = document.querySelector('[data-testid=waveform]') as HTMLElement
          const box = root.getBoundingClientRect()
          const intervals: number[] = []
          const draws: number[] = []
          const levels = new Set<string>()
          let last = 0
          let lastDraw = Number(wave.dataset.drawCount)
          const firstDraw = lastDraw
          let farthest = 0
          let frame = 0
          const FRAMES = 360
          const channel = new MessageChannel()
          const send = (init: WheelEventInit) =>
            root.dispatchEvent(
              new WheelEvent('wheel', { bubbles: true, cancelable: true, clientX: box.left + box.width / 2, clientY: box.top + box.height / 2, ...init }),
            )
          const step = (now: number) => {
            if (last) intervals.push(now - last)
            last = now
            const count = Number(wave.dataset.drawCount)
            if (count !== lastDraw) {
              draws.push(Number(wave.dataset.drawMs))
              levels.add(wave.dataset.level ?? '')
            }
            lastDraw = count
            farthest = Math.max(farthest, Number(root.dataset.from) + box.width * Number(root.dataset.framesPerPixel))
            if (frame < FRAMES) {
              // A wheel event is delivered in a task before the frame it belongs to: a message posted now is handled after this frame.
              const n = frame
              channel.port1.onmessage = () => {
                // Three sweeps over the hour at 16 384 frames a pixel (about 10 500 pixels, 18 frames at 600 px a frame),
                // then zoom in and out over 4 096 times while scrolling.
                if (n < 120) send({ deltaY: Math.floor(n / 20) % 2 === 0 ? 600 : -600 })
                else {
                  send({ ctrlKey: true, deltaY: n % 24 < 12 ? -120 : 120 })
                  send({ deltaY: 300 })
                }
              }
              channel.port2.postMessage(0)
              frame++
              requestAnimationFrame(step)
            } else {
              done({ frames: frame, intervals, draws, drawn: lastDraw - firstDraw, farthest, total: Number(root.dataset.frames), levels: [...levels].sort() })
            }
          }
          requestAnimationFrame(step)
        }),
    )
    const sorted = (values: number[]) => [...values].sort((a, b) => a - b)
    const pct = (values: number[], p: number) => sorted(values)[Math.min(values.length - 1, Math.floor(values.length * p))]!
    const mean = result.intervals.reduce((a, b) => a + b, 0) / result.intervals.length
    const slow = result.intervals.filter((ms) => ms > 20).length / result.intervals.length
    const mode = STRICT ? 'STRICT' : 'ceiling'
    console.log(
      `timing: ${mode} waveform scroll over one hour: ${result.frames} frames, frame interval mean ${mean.toFixed(2)} ms (${(1000 / mean).toFixed(1)} fps), p95 ${pct(result.intervals, 0.95).toFixed(2)} ms, worst ${Math.max(...result.intervals).toFixed(2)} ms, ${(slow * 100).toFixed(1)} % over 20 ms; draw (${result.draws.length} drawn frames) mean ${(result.draws.reduce((a, b) => a + b, 0) / result.draws.length).toFixed(3)} ms, p95 ${pct(result.draws, 0.95).toFixed(3)} ms, worst ${Math.max(...result.draws).toFixed(3)} ms; levels ${result.levels.join(', ')}`,
    )
    // The sweep reached the end of the hour, every level was drawn, and a frame was drawn for most wheel events.
    expect(result.farthest).toBeGreaterThan(result.total * 0.99)
    expect(result.levels).toEqual(['0', '1', '2', '3'])
    expect(result.drawn).toBeGreaterThan(result.frames * 0.9)
    // Ceilings by default (a shared runner is noisy); the screen's pace on a reference machine in strict mode.
    if (STRICT) expect(mean).toBeLessThan(17.5)
    expect(pct(result.draws, 0.95)).toBeLessThan(STRICT ? 4 : 12)
    if (STRICT) expect(slow).toBeLessThan(0.05)
    expect(seen.errors).toEqual([])
  })
})
