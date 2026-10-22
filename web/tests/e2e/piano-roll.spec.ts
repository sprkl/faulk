import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// PN-1 (#82): the piano roll in a real browser. Real worklet, engine and project core: every gesture
// reaches the project as a clip command and a drag is one history entry. The gestures run through
// the pointer; the notes are read back from the roll's data attributes (the project store's view).

const STRICT = process.env.FAULK_E2E_STRICT_TIMING === '1'
if (process.env.FAULK_E2E_STRICT_TIMING !== undefined && !['0', '1'].includes(process.env.FAULK_E2E_STRICT_TIMING)) {
  throw new Error('FAULK_E2E_STRICT_TIMING must be 1 or 0')
}

const SILENCE_DB = -60
const ROW = 16
const RULER = 28

interface Note {
  start: number
  pitch: number
  length: number
  velocity: number
}

const startAudio = (page: Page) => page.getByTestId('engine-state').getByRole('button', { name: /Start audio|Démarrer/ }).click()
const meterDb = async (page: Page) => Number(await page.getByRole('meter').getAttribute('aria-valuenow'))
const roll = (page: Page) => page.getByTestId('piano-roll')
const stage = (page: Page) => page.getByTestId('roll-stage')
const lane = (page: Page) => page.getByTestId('roll-lane')
const notes = async (page: Page): Promise<Note[]> => JSON.parse((await roll(page).getAttribute('data-notes')) ?? '[]')
const count = async (page: Page) => Number(await roll(page).getAttribute('data-note-count'))
const selectedCount = async (page: Page) => Number(await roll(page).getAttribute('data-selected-count'))
const num = async (page: Page, testid: string, attribute: string) => Number(await page.getByTestId(testid).getAttribute(attribute))

/** Where a tick and a pitch are on the page, from the stage's box, the zoom and the vertical scroll. */
async function spot(page: Page) {
  const box = (await stage(page).boundingBox())!
  const ppt = await num(page, 'timegrid', 'data-px-per-tick')
  const scrollY = await num(page, 'roll-stage', 'data-scroll-y')
  return {
    ppt,
    box,
    x: (tick: number) => box.x + tick * ppt,
    /** The middle of the row of a pitch. */
    y: (pitch: number) => box.y + RULER + (127 - pitch) * ROW - scrollY + ROW / 2,
  }
}

async function openRoll(page: Page) {
  const seen = watch(page)
  const warnings: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'warning') warnings.push(message.text())
  })
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  return { seen, warnings }
}

async function createClip(page: Page) {
  await page.getByTestId('roll-create-clip').click()
  await expect(roll(page)).toBeVisible()
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  // The first frame of the grid and its notes is drawn once the surface has a size.
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
}

/** Presses at one place and releases at another, through a few steps, as a drag. */
async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }, hold?: () => Promise<void>) {
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 4 })
  await page.mouse.move(to.x, to.y, { steps: 4 })
  await hold?.()
  await page.mouse.up()
}

test('piano roll: draws, selects, moves, resizes, sets velocity, deletes, sets length and loop, and undoes each as one entry', async ({ page }) => {
  test.setTimeout(120_000)
  const { seen, warnings } = await openRoll(page)

  // A track with no clip offers to create one; with a clip, the empty roll shows its length and no note.
  await expect(page.getByTestId('roll-empty')).toBeVisible()
  await createClip(page)
  await expect(roll(page)).toHaveAttribute('data-clip-length', '3840')
  await page.getByTestId('roll-snap').selectOption('beat')

  // Draw: a click draws a note of the last length (one beat) at the cell of the click.
  let g = await spot(page)
  await page.mouse.click(g.x(1000), g.y(60))
  expect(await notes(page)).toEqual([{ start: 960, pitch: 60, length: 960, velocity: 100 }])
  // Undo takes it back, redo brings it again.
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '0')
  await page.keyboard.press('Control+y')
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')

  // Draw by dragging sets the length, snapped to the beat and kept inside the clip.
  await drag(page, { x: g.x(1990), y: g.y(62) }, { x: g.x(3400), y: g.y(62) })
  expect(await notes(page)).toEqual([
    { start: 960, pitch: 60, length: 960, velocity: 100 },
    { start: 1920, pitch: 62, length: 1920, velocity: 100 },
  ])

  // Select: a rectangle takes the notes it touches, a click on empty space takes none.
  await page.getByTestId('roll-tool-select').click()
  await drag(page, { x: g.x(100), y: g.y(66) }, { x: g.x(3800), y: g.y(56) })
  await expect(roll(page)).toHaveAttribute('data-selected-count', '2')
  await page.mouse.click(g.x(200), g.y(56))
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')
  await page.getByTestId('roll-tool-draw').click()

  // Move: a drag of a note is one history entry (one undo brings it back whole).
  await page.mouse.click(g.x(1100), g.y(60))
  expect(await selectedCount(page)).toBe(1)
  await drag(page, { x: g.x(1100), y: g.y(60) }, { x: g.x(2060), y: g.y(61) })
  // Sorted by start then pitch: the moved note (1920, 61) comes before the other one (1920, 62).
  await expect.poll(async () => (await notes(page))[0]).toEqual({ start: 1920, pitch: 61, length: 960, velocity: 100 })
  expect(await count(page)).toBe(2)
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await notes(page))[0]).toEqual({ start: 960, pitch: 60, length: 960, velocity: 100 })
  expect(await count(page)).toBe(2)

  // Resize: the right edge of a note, snapped to the beat.
  await drag(page, { x: g.x(1910), y: g.y(60) }, { x: g.x(2900), y: g.y(60) })
  await expect.poll(async () => (await notes(page))[0]).toEqual({ start: 960, pitch: 60, length: 1920, velocity: 100 })

  // Velocity: the bar of the selected note in the lane, dragged to the middle of the lane.
  const laneBox = (await lane(page).boundingBox())!
  const barX = g.x(960) + 1
  await drag(page, { x: barX, y: laneBox.y + 13 }, { x: barX, y: laneBox.y + 28 })
  await expect.poll(async () => Math.abs((await notes(page))[0]!.velocity - 64)).toBeLessThanOrEqual(1)
  const afterVelocity = await notes(page)

  // Delete: Delete removes the selected note, undo brings it back.
  await page.mouse.click(g.x(2500), g.y(62))
  expect(await selectedCount(page)).toBe(1)
  await page.keyboard.press('Delete')
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-note-count', '2')
  // Escape clears the selection.
  await page.keyboard.press('Escape')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '0')

  // An undo while a drag is open ends the drag and sends nothing: no refused command.
  await page.mouse.click(g.x(1100), g.y(60))
  await drag(page, { x: g.x(1100), y: g.y(60) }, { x: g.x(1400), y: g.y(63) }, async () => {
    await page.keyboard.press('Control+z')
  })
  await expect.poll(async () => (await notes(page))[0]!.velocity).toBe(100)
  expect(await count(page)).toBe(2)
  expect((await notes(page))[0]).toMatchObject({ start: 960, pitch: 60, length: 1920 })
  expect(afterVelocity[0]!.velocity).not.toBe(100)
  expect(warnings.filter((w) => w.includes('refused'))).toEqual([])

  // Length: one bar longer, undone, then typed in bars.
  await page.getByTestId('roll-longer').click()
  await expect(roll(page)).toHaveAttribute('data-clip-length', '7680')
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-clip-length', '3840')
  await page.getByTestId('roll-length').fill('3')
  await page.getByTestId('roll-length').press('Tab')
  await expect(roll(page)).toHaveAttribute('data-clip-length', '11520')
  // Backspace in the field edits the number and never deletes notes.
  await page.getByTestId('roll-length').press('Backspace')
  expect(await count(page)).toBe(2)

  // Loop: the whole clip by the button, a part of it by dragging on the ruler.
  await page.getByTestId('roll-loop').click()
  await expect(roll(page)).toHaveAttribute('data-loop', '0-11520')
  await page.getByTestId('roll-loop').click()
  await expect(roll(page)).toHaveAttribute('data-loop', '')
  g = await spot(page)
  await drag(page, { x: g.x(960), y: g.box.y + 10 }, { x: g.x(1920), y: g.box.y + 10 })
  await expect(roll(page)).toHaveAttribute('data-loop', '960-1920')
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-loop', '')

  // Audition: a note pressed on the grid sounds through the track while the pointer is down.
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBe(SILENCE_DB)
  await page.mouse.move(g.x(100), g.y(58))
  await page.mouse.down()
  await expect.poll(() => meterDb(page), { timeout: 10_000 }).toBeGreaterThan(SILENCE_DB)
  await page.mouse.up()
  await expect.poll(() => count(page)).toBe(3)
  // Released: the note stops (the meter falls back to silence).
  await expect.poll(() => meterDb(page), { timeout: 15_000 }).toBe(SILENCE_DB)

  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/piano-roll.png` })
  expect(seen.errors).toEqual([])
})

test('piano roll: the empty states and the toolbar speak the page language', async ({ page }) => {
  const seen = watch(page)
  await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('roll-create-clip')).toHaveText('Créer un clip')
  await createClip(page)
  await expect(page.getByTestId('roll-tool-draw')).toHaveText('Dessiner')
  await expect(page.getByTestId('roll-tool-select')).toHaveText('Sélectionner')
  await expect(page.getByTestId('roll-loop')).toHaveText('Boucle')
  await expect(page.getByTestId('roll-count')).toHaveText('0 notes')
  expect(seen.errors).toEqual([])
})

// 2 000 notes: the cost of a frame while scrolling, zooming and dragging them all, and of a command
// that changes all of them. The page's dev hook (`__faulkRun`, the dispatcher's one entry point) loads them.
test('piano roll: stays fluid with 2 000 notes', async ({ page }) => {
  test.setTimeout(180_000)
  const { seen, warnings } = await openRoll(page)
  const hook = await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
  test.skip(hook !== 'function', 'the dispatcher hook exists in the dev server only')
  await createClip(page)
  const clip = (await roll(page).getAttribute('data-clip'))!

  // 50 bars, 2 000 notes: 40 a bar, on 40 distinct rows over 4 octaves, no two alike.
  const BARS = 50
  await page.evaluate(
    ({ clip, bars }) => {
      const run = (window as unknown as { __faulkRun: (c: unknown) => void }).__faulkRun
      const notes = Array.from({ length: 2000 }, (_, i) => ({ start: Math.floor(i / 40) * 3840 + (i % 8) * 480, pitch: 48 + ((i * 7) % 24), length: 360, velocity: 40 + (i % 80) }))
      run({ type: 'clip.set_content', clip, length: bars * 3840, looping: null, notes })
    },
    { clip, bars: BARS },
  )
  await expect(roll(page)).toHaveAttribute('data-note-count', '2000')
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(1)
  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/piano-roll-2000.png` })

  // Sweep: scroll over the clip and zoom in and out, one wheel event a frame; each drawn frame costs grid + notes.
  const sweep = () =>
    page.evaluate(
      () =>
        new Promise<{ frames: number; intervals: number[]; grid: number[]; body: number[]; scrolled: number }>((done) => {
          const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
          const box = root.getBoundingClientRect()
          const intervals: number[] = []
          const grid: number[] = []
          const body: number[] = []
          let last = 0
          let lastDraw = Number(root.dataset.drawCount)
          let farthest = 0
          let frame = 0
          const FRAMES = 240
          const channel = new MessageChannel()
          const send = (init: WheelEventInit) => root.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, clientX: box.left + 300, clientY: box.top + 150, ...init }))
          const step = (now: number) => {
            if (last) intervals.push(now - last)
            last = now
            const drawn = Number(root.dataset.drawCount)
            if (drawn !== lastDraw) {
              grid.push(Number(root.dataset.drawMs))
              body.push(Number(root.dataset.bodyMs))
            }
            lastDraw = drawn
            farthest = Math.max(farthest, Number(root.dataset.scrollTick))
            if (frame < FRAMES) {
              const n = frame
              channel.port1.onmessage = () => {
                if (n < 160) send({ shiftKey: true, deltaY: n < 80 ? 700 : -700 })
                else {
                  send({ ctrlKey: true, deltaY: n % 16 < 8 ? -120 : 120 })
                  send({ shiftKey: true, deltaY: 300 })
                }
              }
              channel.port2.postMessage(0)
              frame++
              requestAnimationFrame(step)
            } else done({ frames: frame, intervals, grid, body, scrolled: farthest })
          }
          requestAnimationFrame(step)
        }),
    )
  const sorted = (values: number[]) => [...values].sort((a, b) => a - b)
  const pct = (values: number[], p: number) => sorted(values)[Math.min(values.length - 1, Math.floor(values.length * p))]!
  const mean = (values: number[]) => values.reduce((a, b) => a + b, 0) / values.length
  const total = (r: { grid: number[]; body: number[] }) => r.grid.map((v, i) => v + r.body[i]!)

  const swept = await sweep()
  const cost = total(swept)
  const mode = STRICT ? 'STRICT' : 'ceiling'
  console.log(
    `timing: ${mode} piano roll, 2000 notes, scroll and zoom: ${swept.frames} frames, frame interval mean ${mean(swept.intervals).toFixed(2)} ms, p95 ${pct(swept.intervals, 0.95).toFixed(2)} ms, worst ${Math.max(...swept.intervals).toFixed(2)} ms; draw (${cost.length} drawn frames, grid + notes) mean ${mean(cost).toFixed(3)} ms, p95 ${pct(cost, 0.95).toFixed(3)} ms, worst ${Math.max(...cost).toFixed(3)} ms (notes alone: mean ${mean(swept.body).toFixed(3)} ms, p95 ${pct(swept.body, 0.95).toFixed(3)} ms)`,
  )
  expect(cost.length).toBeGreaterThan(swept.frames * 0.8)
  expect(mean(swept.intervals)).toBeLessThan(STRICT ? 17.5 : 25)
  expect(pct(cost, 0.95)).toBeLessThan(STRICT ? 4 : 12)

  // A drag of all the notes at once, by the pointer: the cost of the frames drawn while it moves.
  // Zoomed out to the whole clip, so that all 2 000 notes are in sight: the worst case for a frame.
  await page.evaluate(() => {
    const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
    const box = root.getBoundingClientRect()
    root.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, ctrlKey: true, deltaY: 20000, clientX: box.left + 10, clientY: box.top + 100 }))
  })
  await expect.poll(async () => (await spot(page)).ppt).toBeLessThan(0.006)
  await page.getByTestId('timegrid').press('Home')
  await page.getByTestId('timegrid').press('Control+a')
  await expect(roll(page)).toHaveAttribute('data-selected-count', '2000')
  const g = await spot(page)
  // A note in sight at the start of the clip: the one at tick 1920 on pitch 60 (the roll opens around middle C).
  const target = { x: g.x(1920) + 2, y: g.y(60) }
  const firstBefore = await roll(page).getAttribute('data-first-note')
  // From here on, every animation frame is sampled: its interval, the grid and notes draw, and the velocity lane draw.
  await page.evaluate(() => {
    const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
    const laneRoot = document.querySelector('[data-testid=roll-lane] [data-draw-ms]') as HTMLElement
    const samples = { intervals: [] as number[], cost: [] as number[], lane: [] as number[] }
    let last = 0
    let lastDraw = Number(root.dataset.drawCount)
    let lastLane = laneRoot.dataset.drawMs
    const tick = (now: number) => {
      if (last) samples.intervals.push(now - last)
      last = now
      const drawn = Number(root.dataset.drawCount)
      if (drawn !== lastDraw) samples.cost.push(Number(root.dataset.drawMs) + Number(root.dataset.bodyMs))
      lastDraw = drawn
      if (laneRoot.dataset.drawMs !== lastLane) samples.lane.push(Number(laneRoot.dataset.drawMs))
      lastLane = laneRoot.dataset.drawMs
      ;(window as unknown as { __samples: typeof samples }).__samples = samples
      requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)
  })
  await page.mouse.move(target.x, target.y)
  await page.mouse.down()
  for (let i = 1; i <= 40; i++) {
    await page.mouse.move(target.x + i * 5, target.y - Math.floor(i / 10) * ROW)
    // One move a frame, as a hand does.
    await page.waitForTimeout(16)
  }
  const dragged = await page.evaluate(() => (window as unknown as { __samples: { intervals: number[]; cost: number[]; lane: number[] } }).__samples)
  const dragCost = dragged.cost
  await page.mouse.up()
  console.log(
    `timing: ${mode} piano roll, 2000 notes all in sight, drag of all of them: ${dragCost.length} drawn frames, grid + notes mean ${mean(dragCost).toFixed(3)} ms, p95 ${pct(dragCost, 0.95).toFixed(3)} ms, worst ${Math.max(...dragCost).toFixed(3)} ms; velocity lane (${dragged.lane.length} draws) mean ${mean(dragged.lane).toFixed(3)} ms, p95 ${pct(dragged.lane, 0.95).toFixed(3)} ms; frame interval mean ${mean(dragged.intervals).toFixed(2)} ms, p95 ${pct(dragged.intervals, 0.95).toFixed(2)} ms, worst ${Math.max(...dragged.intervals).toFixed(2)} ms (handlers included)`,
  )
  expect(dragCost.length).toBeGreaterThan(10)
  expect(pct(dragCost, 0.95)).toBeLessThan(STRICT ? 4 : 12)
  expect(pct(dragged.lane, 0.95)).toBeLessThan(STRICT ? 4 : 12)
  expect(mean(dragged.intervals)).toBeLessThan(STRICT ? 17.5 : 25)
  // The release sent one command for the 2 000 notes: they moved (the first one is not where it was) as one entry, one undo takes them back.
  await expect(roll(page)).toHaveAttribute('data-note-count', '2000')
  await expect.poll(() => roll(page).getAttribute('data-first-note')).not.toBe(firstBefore)
  await page.keyboard.press('Control+z')
  await expect.poll(() => roll(page).getAttribute('data-first-note')).toBe(firstBefore)
  await expect(roll(page)).toHaveAttribute('data-note-count', '2000')

  // The command that changes all 2 000 notes: from the call to the frame that shows the result.
  const roundTrip = await page.evaluate(
    ({ clip }) =>
      new Promise<number>((done) => {
        const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
        const run = (window as unknown as { __faulkRun: (c: unknown) => void }).__faulkRun
        const before = Number(root.dataset.drawCount)
        const notes = Array.from({ length: 2000 }, (_, i) => ({ start: Math.floor(i / 40) * 3840 + (i % 8) * 480, pitch: 48 + ((i * 7) % 24), length: 360, velocity: 40 + (i % 80) }))
        const changes = notes.map((from) => ({ from, to: { ...from, velocity: from.velocity === 127 ? 126 : from.velocity + 1 } }))
        const t0 = performance.now()
        run({ type: 'clip.notes.update', clip, changes })
        const wait = () => (Number(root.dataset.drawCount) > before ? done(performance.now() - t0) : requestAnimationFrame(wait))
        wait()
      }),
    { clip },
  )
  console.log(`timing: ${mode} piano roll, clip.notes.update of 2000 notes, call to drawn frame: ${roundTrip.toFixed(1)} ms`)
  expect(roundTrip).toBeLessThan(STRICT ? 100 : 1000)

  expect(warnings.filter((w) => w.includes('refused'))).toEqual([])
  expect(seen.errors).toEqual([])
})
