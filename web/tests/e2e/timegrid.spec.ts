import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// AR-3 (#79): the shared time grid in a real browser, on its test surface (`/dev/timegrid`, served
// by the dev server and by the e2e build, which sets VITE_FAULK_DEV_ROUTES). Zoom, scroll, snapping,
// the canvas itself (pixels read back) and the cost of scrolling over 200 bars.

const STRICT = process.env.FAULK_E2E_STRICT_TIMING === '1'
if (process.env.FAULK_E2E_STRICT_TIMING !== undefined && !['0', '1'].includes(process.env.FAULK_E2E_STRICT_TIMING)) {
  throw new Error('FAULK_E2E_STRICT_TIMING must be 1 or 0')
}

const PPQ = 960
const BAR = PPQ * 4
const TOTAL = 200 * BAR

const grid = (page: Page) => page.getByTestId('timegrid')
const num = async (page: Page, attribute: string) => Number(await grid(page).getAttribute(attribute))
const scrollTick = (page: Page) => num(page, 'data-scroll-tick')
const zoom = (page: Page) => num(page, 'data-px-per-tick')
const hoverTick = async (page: Page) => Number(await page.getByTestId('hover').getAttribute('data-tick'))

async function open(page: Page) {
  const seen = watch(page)
  await page.goto('/dev/timegrid')
  await expect(grid(page)).toBeVisible()
  // The first frame is drawn on the next animation frame after the size is known.
  await expect.poll(async () => Number(await grid(page).getAttribute('data-draw-count'))).toBeGreaterThan(0)
  return seen
}

/** Moves the pointer to `x` pixels from the grid's left edge, `y` from its top, and returns the box. */
async function pointAt(page: Page, x: number, y = 100) {
  const box = (await grid(page).boundingBox())!
  await page.mouse.move(box.x + x, box.y + y)
  return box
}

/** The colour of a canvas pixel in device pixels. */
/** One pixel of the grid's canvas, or of the overlay canvas that carries the play position. */
const pixel = (page: Page, x: number, y: number, selector = 'canvas:not(.overlay)') =>
  page.evaluate(
    ([px, py, sel]) => {
      const canvas = document.querySelector(`[data-testid=timegrid] ${sel}`) as HTMLCanvasElement
      return Array.from(canvas.getContext('2d')!.getImageData(Number(px), Number(py), 1, 1).data)
    },
    [x, y, selector] as const,
  )

test.describe('time grid', () => {
  test('draws the ruler and the bar lines on its canvas', async ({ page }) => {
    const seen = await open(page)
    // Default zoom: 0.1 px per tick, a bar is 384 px; the bar line sits on pixel 384 (dpr 1).
    const dpr = await page.evaluate(() => window.devicePixelRatio)
    const onLine = await pixel(page, Math.round(384 * dpr), Math.round(100 * dpr))
    const between = await pixel(page, Math.round(390 * dpr), Math.round(100 * dpr))
    expect(onLine).not.toEqual(between)
    const rulerBand = await pixel(page, Math.round(190 * dpr), Math.round(4 * dpr))
    expect(rulerBand).not.toEqual(between)
    // Screenshots go to the folder named by FAULK_E2E_SHOTS, when set.
    if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/timegrid.png` })
    expect(seen.errors).toEqual([])
  })

  test('draws again after a resize and after a change of the pixel ratio', async ({ page }) => {
    const seen = await open(page)
    const canvasSize = () => page.evaluate(() => {
      const el = document.querySelector('[data-testid=timegrid] canvas') as HTMLCanvasElement
      return { width: el.width, height: el.height, css: (el.parentElement as HTMLElement).getBoundingClientRect().width, dpr: window.devicePixelRatio }
    })
    const lineVisible = async (dpr: number) => {
      const onLine = await pixel(page, Math.round(384 * dpr), Math.round(100 * dpr))
      const between = await pixel(page, Math.round(390 * dpr), Math.round(100 * dpr))
      return JSON.stringify(onLine) !== JSON.stringify(between)
    }
    await page.setViewportSize({ width: 900, height: 500 })
    await expect.poll(async () => (await canvasSize()).width).toBe(900)
    await expect.poll(() => lineVisible(1)).toBe(true)
    // The pixel ratio goes to 2 with a new size (the emulation fires no media query change when only the ratio moves,
    // so the listener for a monitor change is not exercised here): the bitmap follows both.
    const cdp = await page.context().newCDPSession(page)
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: 880, height: 500, deviceScaleFactor: 2, mobile: false })
    await expect.poll(async () => (await canvasSize()).width).toBe(1760)
    await expect.poll(() => lineVisible(2)).toBe(true)
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: 900, height: 500, deviceScaleFactor: 1, mobile: false })
    await expect.poll(async () => (await canvasSize()).width).toBe(900)
    await expect.poll(() => lineVisible(1)).toBe(true)
    expect(seen.errors).toEqual([])
  })

  test('zooms with Ctrl + wheel around the pointer and stops at both ends', async ({ page }) => {
    const seen = await open(page)
    await page.getByTestId('snap').selectOption('off')
    await pointAt(page, 300)
    const before = await hoverTick(page)
    const z0 = await zoom(page)
    await page.keyboard.down('Control')
    await page.mouse.wheel(0, -300)
    await expect.poll(() => zoom(page)).toBeGreaterThan(z0)
    await page.keyboard.up('Control')
    // The tick under the pointer stayed under it.
    await pointAt(page, 301)
    await pointAt(page, 300)
    expect(Math.abs((await hoverTick(page)) - before)).toBeLessThanOrEqual(2 / (await zoom(page)))
    await page.keyboard.down('Control')
    for (let i = 0; i < 40; i++) await page.mouse.wheel(0, -500)
    await expect.poll(() => zoom(page)).toBe(1)
    for (let i = 0; i < 80; i++) await page.mouse.wheel(0, 500)
    const width = (await grid(page).boundingBox())!.width
    await expect.poll(async () => (await zoom(page)) * TOTAL).toBeCloseTo(Math.floor(width), -1)
    await page.keyboard.up('Control')
    expect(await scrollTick(page)).toBe(0)
    expect(seen.errors).toEqual([])
  })

  test('scrolls with Shift + wheel and the keyboard, and stops at both ends', async ({ page }) => {
    const seen = await open(page)
    await pointAt(page, 300)
    await page.keyboard.down('Shift')
    await page.mouse.wheel(0, 400)
    await page.keyboard.up('Shift')
    await expect.poll(() => scrollTick(page)).toBe(4000)
    await page.mouse.wheel(200, 0)
    await expect.poll(() => scrollTick(page)).toBe(6000)
    // The still pointer (300 px) is now over tick 9000: the readout follows the scroll.
    await expect.poll(() => hoverTick(page)).toBeGreaterThan(8900)
    expect(await hoverTick(page)).toBeLessThan(9100)
    // A plain vertical wheel is left to the host.
    await page.mouse.wheel(0, 400)
    await page.waitForTimeout(100)
    expect(await scrollTick(page)).toBe(6000)
    await grid(page).focus()
    await page.keyboard.press('ArrowRight')
    const width = (await grid(page).boundingBox())!.width
    expect(await scrollTick(page)).toBeCloseTo(6000 + (width / 10) / 0.1, 3)
    await page.keyboard.press('ArrowLeft')
    await expect.poll(() => scrollTick(page)).toBeCloseTo(6000, 3)
    await page.keyboard.press('Home')
    await expect.poll(() => scrollTick(page)).toBe(0)
    await page.keyboard.press('ArrowLeft')
    expect(await scrollTick(page)).toBe(0)
    await page.keyboard.down('Shift')
    await page.mouse.wheel(0, 10_000_000)
    await page.keyboard.up('Shift')
    await expect.poll(async () => (await scrollTick(page)) + width / (await zoom(page))).toBeCloseTo(TOTAL, 3)
    await page.keyboard.press('+')
    await expect.poll(() => zoom(page)).toBeCloseTo(0.125, 6)
    await page.keyboard.press('-')
    await expect.poll(() => zoom(page)).toBeCloseTo(0.1, 6)
    expect(seen.errors).toEqual([])
  })

  test('snaps the pointer to the chosen unit, and to the signature', async ({ page }) => {
    const seen = await open(page)
    // 200 px at 0.1 px per tick is tick 2000; a bar is 3840 ticks, a beat 960.
    const expected = { off: 2000, auto: 2040, bar: 3840, beat: 1920, 'half-beat': 1920, 'quarter-beat': 1920 }
    for (const [setting, tick] of Object.entries(expected)) {
      await page.getByTestId('snap').selectOption(setting)
      await pointAt(page, 199)
      await pointAt(page, 200)
      expect(await hoverTick(page), setting).toBe(tick)
    }
    // 3/8: a beat of 480 ticks, a bar of 1440; 215 px is tick 2150.
    await page.getByTestId('numerator').fill('3')
    await page.getByTestId('denominator').selectOption('8')
    for (const [setting, tick] of Object.entries({ bar: 1440, beat: 1920, 'half-beat': 2160 })) {
      await page.getByTestId('snap').selectOption(setting)
      await pointAt(page, 214)
      await pointAt(page, 215)
      expect(await hoverTick(page), `3/8 ${setting}`).toBe(tick)
    }
    // The pointer leaving the grid clears the readout.
    await page.mouse.move(5, 5)
    await expect(page.getByTestId('hover')).toHaveText('-')
    expect(seen.errors).toEqual([])
  })

  test('a click on the ruler seeks to the snapped tick and draws the play position', async ({ page }) => {
    const seen = await open(page)
    await page.getByTestId('snap').selectOption('beat')
    const box = (await grid(page).boundingBox())!
    // The body does not seek.
    await page.mouse.click(box.x + 200, box.y + 120)
    await expect(page.getByTestId('seeked')).toHaveText('-')
    await page.mouse.click(box.x + 200, box.y + 10)
    await expect(page.getByTestId('seeked')).toHaveAttribute('data-tick', '1920')
    await expect(page.getByTestId('seeked')).toHaveText('1.3.000')
    const dpr = await page.evaluate(() => window.devicePixelRatio)
    // Beat 1920 is at 192 px: the play colour (green), 2 px wide.
    await expect
      .poll(async () => {
        const [r, g, b] = await pixel(page, Math.round(192 * dpr), Math.round(120 * dpr), 'canvas.overlay')
        return g > r && g > b
      })
      .toBe(true)
    expect(seen.errors).toEqual([])
  })

  test('scrolls over 200 bars at the pace of the screen', async ({ page }) => {
    test.setTimeout(120_000)
    const seen = await open(page)
    const result = await page.evaluate(
      ({ total }) =>
        new Promise<{ frames: number; intervals: number[]; draws: number[]; drawn: number; scrolled: number; zoomed: number }>((done) => {
          const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
          const box = root.getBoundingClientRect()
          const intervals: number[] = []
          const draws: number[] = []
          let last = 0
          let lastDraw = Number(root.dataset.drawCount)
          const firstDraw = lastDraw
          let farthest = 0
          let frame = 0
          let nearest = 1
          const FRAMES = 360
          const channel = new MessageChannel()
          const send = (init: WheelEventInit) =>
            root.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, clientX: box.left + 500, clientY: box.top + 200, ...init }))
          const step = (now: number) => {
            if (last) intervals.push(now - last)
            last = now
            const count = Number(root.dataset.drawCount)
            if (count !== lastDraw) draws.push(Number(root.dataset.drawMs))
            lastDraw = count
            farthest = Math.max(farthest, Number(root.dataset.scrollTick))
            nearest = Math.min(nearest, Number(root.dataset.pxPerTick))
            if (frame < FRAMES) {
              // The browser delivers a wheel event in a task before the frame it belongs to, not inside the animation
              // frame callbacks: a message posted now is handled after this frame and before the next one.
              const n = frame
              channel.port1.onmessage = () => {
                // Two sweeps over the 200 bars (about 2 x 128 frames at 600 px a frame), then zoom in and out while scrolling.
                if (n < 256) send({ shiftKey: true, deltaY: Math.floor(n / 128) % 2 === 0 ? 600 : -600 })
                else {
                  send({ ctrlKey: true, deltaY: n % 16 < 8 ? -120 : 120 })
                  send({ shiftKey: true, deltaY: 300 })
                }
              }
              channel.port2.postMessage(0)
              frame++
              requestAnimationFrame(step)
            } else {
              done({ frames: frame, intervals, draws, drawn: lastDraw - firstDraw, scrolled: farthest, zoomed: nearest })
            }
          }
          requestAnimationFrame(step)
          void total
        }),
      { total: TOTAL },
    )
    const sorted = (values: number[]) => [...values].sort((a, b) => a - b)
    const pct = (values: number[], p: number) => sorted(values)[Math.min(values.length - 1, Math.floor(values.length * p))]
    const mean = result.intervals.reduce((a, b) => a + b, 0) / result.intervals.length
    const slow = result.intervals.filter((ms) => ms > 20).length / result.intervals.length
    const mode = STRICT ? 'STRICT' : 'ceiling'
    console.log(
      `timing: ${mode} timegrid scroll over 200 bars: ${result.frames} frames, frame interval mean ${mean.toFixed(2)} ms (${(1000 / mean).toFixed(1)} fps), p95 ${pct(result.intervals, 0.95).toFixed(2)} ms, worst ${Math.max(...result.intervals).toFixed(2)} ms, ${(slow * 100).toFixed(1)} % over 20 ms; draw (${result.draws.length} drawn frames) mean ${(result.draws.reduce((a, b) => a + b, 0) / result.draws.length).toFixed(3)} ms, p95 ${pct(result.draws, 0.95).toFixed(3)} ms, worst ${Math.max(...result.draws).toFixed(3)} ms; furthest scroll tick ${Math.round(result.scrolled)} of ${TOTAL}`,
    )
    // The sweep really covered the grid, and a frame was drawn for the great majority of the wheel events.
    expect(result.scrolled).toBeGreaterThan(TOTAL * 0.9)
    expect(result.drawn).toBeGreaterThan(result.frames * 0.9)
    // Ceilings by default (a shared runner is noisy); the screen's pace itself on a reference machine.
    // The frame interval follows the runner, not the code: held in strict mode only; the draw cost below is held everywhere.
    if (STRICT) expect(mean).toBeLessThan(17.5)
    expect(pct(result.draws, 0.95)).toBeLessThan(STRICT ? 4 : 12)
    if (STRICT) expect(slow).toBeLessThan(0.05)
    expect(seen.errors).toEqual([])
  })
})
