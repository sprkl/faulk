import { expect, test, type Page } from '@playwright/test'
import { fakeWebMidi, STRICT_TIMING, watch } from './support'

// AM-4 (#121): the automation lanes in the arrangement timeline, in a real browser with the real worklet, engine and
// project core. Every gesture is a command (a refused command would be a console warning, which the tests forbid), a
// drag is one history entry, the controls follow the lane while the transport runs, and the engine really plays it
// (the master's level, which is the engine's report, follows a volume lane).

const LANE_PAD = 10
const LANE_ROW = 84
const BAR = 3840

interface Lane {
  target: Record<string, unknown>
  points: Record<string, { value: number; shape: string }>
}
type View = { project: { tracks: Record<string, { automation: Record<string, Lane>; devices: Record<string, { source: { builtin?: { id: string } } }> }> } }

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
const arr = (page: Page) => page.getByTestId('arrangement')
const stage = (page: Page) => page.getByTestId('arr-stage')
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
const project = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView().project)
const tracks = async (page: Page) => Object.entries((await project(page)).tracks)
/** The lanes of the first track, by id. */
const lanes = async (page: Page): Promise<[string, Lane][]> => Object.entries((await tracks(page))[0]![1].automation)
const pointsOf = (lane: Lane) =>
  Object.entries(lane.points)
    .map(([tick, p]) => ({ tick: Number(tick), value: p.value, shape: p.shape }))
    .sort((a, b) => a.tick - b.tick)
const master = async (page: Page) => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))

/** Where a tick and a value of a lane are on the page. */
async function spot(page: Page, lane: number) {
  const box = (await stage(page).boundingBox())!
  const ppt = Number(await stage(page).getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const head = (await page.getByTestId('arr-lane').nth(lane).boundingBox())!
  return {
    x: (tick: number) => box.x + tick * ppt,
    // The position of a value along the lane's axis (0 bottom, 1 top).
    y: (position: number) => head.y + LANE_PAD + (1 - position) * (LANE_ROW - 2 * LANE_PAD),
    top: head.y,
    ppt,
    box,
  }
}

async function open(page: Page, width = 1600, midi = false) {
  await page.setViewportSize({ width, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  if (midi) await page.getByRole('radio', { name: 'E2E Keys' }).check()
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toBeVisible()
  await expect.poll(async () => Number(await stage(page).getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
}

/** Unfolds the first track and adds a lane for the parameter named `label` (the menu's option). */
async function addLane(page: Page, label: string) {
  const toggle = page.getByTestId('arr-automation-toggle').first()
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click()
  const before = await page.getByTestId('arr-lane').count()
  await page.getByTestId('arr-add-lane').first().selectOption({ label })
  await expect(page.getByTestId('arr-lane')).toHaveCount(before + 1)
}

test('automation: three points drawn by clicks, played into the device panel, edited, and undone', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  const refused: string[] = []
  page.on('console', (message) => {
    if (message.type() === 'warning' && /refused/.test(message.text())) refused.push(message.text())
  })
  await open(page)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByTestId('panel-devices').click()
  await page.getByTestId('arr-snap').selectOption('beat')
  await expect(page.getByTestId('arr-lane')).toHaveCount(0)

  // The lanes are folded away until the track's toggle opens them; a lane is made by choosing its parameter.
  await expect(page.getByTestId('arr-automation-toggle').first()).toHaveAttribute('aria-expanded', 'false')
  await addLane(page, 'Gain')
  expect(await lanes(page)).toHaveLength(1)
  await expect(page.getByTestId('arr-lane').first()).toHaveAttribute('data-target', 'Oscillator › Gain')
  // A parameter that has its lane is no longer offered.
  await expect(page.getByTestId('arr-add-lane').first().locator('option', { hasText: 'Gain' })).toHaveCount(0)
  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/lane-empty.png` })

  // Three clicks draw three points: a click on an empty part is one `automation.points.add`.
  await stage(page).getByTestId('timegrid').focus()
  for (const [tick, value] of [
    [0, 0.9],
    [BAR / 2, 0.2],
    [BAR, 0.9],
  ] as const) {
    // The page may move a little between two clicks (the top bar changes its text): the spots are read again each time.
    const g = await spot(page, 0)
    await page.mouse.click(g.x(tick) + 1, g.y(value))
  }
  const [laneId, lane] = (await lanes(page))[0]!
  expect(pointsOf(lane).map((p) => p.tick)).toEqual([0, BAR / 2, BAR])
  pointsOf(lane).forEach((p, i) => expect(p.value).toBeCloseTo([0.9, 0.2, 0.9][i]!, 1))
  expect(pointsOf(lane).every((p) => p.shape === 'linear')).toBe(true)
  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/lane-three-points.png` })

  // Play: the Gain knob of the device panel moves with the lane (the engine plays the lane; the panel shows it).
  const gain = page.getByTestId('device-card').first().getByTestId('param-gain').getByRole('slider')
  const stored = await gain.getAttribute('aria-valuetext')
  await page.getByTestId('transport-play').click()
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  const seenTexts = new Set<string>()
  const started = Date.now()
  while (Date.now() - started < 3000) {
    seenTexts.add((await gain.getAttribute('aria-valuetext')) ?? '')
    await page.waitForTimeout(40)
  }
  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/lane-playing.png` })
  await page.getByTestId('transport-stop').click()
  expect(seenTexts.size, `the knob showed ${[...seenTexts].join(' | ')}`).toBeGreaterThan(5)
  // Stopped, the control shows the project's value again.
  await expect(gain).toHaveAttribute('aria-valuetext', stored!)

  // A drag of the middle point is ONE group: one undo gives the lane back as it was. (The page may have moved while playing: read the spots again.)
  let h = await spot(page, 0)
  const before = pointsOf((await lanes(page))[0]![1])
  await stage(page).getByTestId('timegrid').focus()
  await page.mouse.move(h.x(BAR / 2), h.y(0.2))
  await page.mouse.down()
  await page.mouse.move(h.x(BAR / 2 + 480), h.y(0.5), { steps: 6 })
  await page.mouse.move(h.x(BAR / 2 + 960), h.y(0.6), { steps: 6 })
  await page.mouse.up()
  const dragged = pointsOf((await lanes(page))[0]![1])
  expect(dragged.map((p) => p.tick)).toEqual([0, BAR / 2 + 960, BAR])
  expect(dragged[1]!.value).toBeCloseTo(0.6, 1)
  await page.keyboard.press('Control+z')
  expect(pointsOf((await lanes(page))[0]![1])).toEqual(before)
  await page.keyboard.press('Control+Shift+z')
  expect(pointsOf((await lanes(page))[0]![1]).map((p) => p.tick)).toEqual([0, BAR / 2 + 960, BAR])
  await page.keyboard.press('Control+z')

  // A value drag keeps the tick: one `points.update`. A step curve is set with the toolbar.
  h = await spot(page, 0)
  await page.mouse.move(h.x(BAR), h.y(0.9))
  await page.mouse.down()
  await page.mouse.move(h.x(BAR), h.y(0.4), { steps: 5 })
  await page.mouse.up()
  expect(pointsOf((await lanes(page))[0]![1])[2]).toMatchObject({ tick: BAR, shape: 'linear' })
  expect(pointsOf((await lanes(page))[0]![1])[2]!.value).toBeCloseTo(0.4, 1)
  await expect(page.getByTestId('arr-curve-step')).toBeEnabled()
  await page.getByTestId('arr-curve-step').click()
  expect(pointsOf((await lanes(page))[0]![1])[2]!.shape).toBe('step')
  await page.getByTestId('arr-curve-linear').click()
  expect(pointsOf((await lanes(page))[0]![1])[2]!.shape).toBe('linear')

  // A box selects several points; the context menu sets their curve and deletes them; Delete deletes too.
  h = await spot(page, 0)
  await page.mouse.move(h.x(BAR + 480), h.y(0))
  await page.mouse.down()
  await page.mouse.move(h.x(0) - 10, h.y(1) - 6, { steps: 6 })
  await page.mouse.up()
  await expect(page.getByTestId('arr-count')).toContainText('3')
  await page.mouse.click(h.x(BAR / 2), h.y(0.2), { button: 'right' })
  await expect(page.getByTestId('point-menu')).toBeVisible()
  await page.getByTestId('point-menu-step').click()
  expect(pointsOf((await lanes(page))[0]![1]).every((p) => p.shape === 'step')).toBe(true)
  await page.keyboard.press('Delete')
  expect(pointsOf((await lanes(page))[0]![1])).toHaveLength(0)
  await page.keyboard.press('Control+z')
  expect(pointsOf((await lanes(page))[0]![1])).toHaveLength(3)

  // Undo removes the three points we drew (and the step curve), one entry each, then the lane itself.
  await page.keyboard.press('Control+z') // the step curve of the menu
  await page.keyboard.press('Control+z') // the curve back to linear
  await page.keyboard.press('Control+z') // the step curve
  await page.keyboard.press('Control+z') // the value drag
  for (let i = 0; i < 3; i++) await page.keyboard.press('Control+z')
  expect(pointsOf((await lanes(page))[0]![1])).toHaveLength(0)
  await page.keyboard.press('Control+z')
  expect(await lanes(page)).toHaveLength(0)
  await expect(page.getByTestId('arr-lane')).toHaveCount(0)
  void laneId

  expect(refused).toEqual([])
  expect(seen.errors).toEqual([])
})

test('automation: a typed value, a deleted lane, and a lane that the engine really plays (volume)', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await fakeWebMidi(page)
  await open(page, 1600, true)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByTestId('arr-snap').selectOption('beat')
  await addLane(page, 'Volume')
  await expect(page.getByTestId('arr-lane').first()).toHaveAttribute('data-target', 'Mixer › Volume')
  const [laneId] = (await lanes(page))[0]!
  // Held at 0 dB for a bar, then -60 dB.
  await run(page, {
    type: 'automation.points.add',
    lane: laneId,
    points: [
      { tick: 0, value: 0, shape: 'step' },
      { tick: BAR, value: -60, shape: 'step' },
    ],
  })
  await expect(page.getByTestId('arr-lane').first()).toHaveAttribute('data-points', '2')

  // A double click on a point opens the field to type its value: -12 dB is one `points.update`.
  await stage(page).getByTestId('timegrid').focus()
  const g = await spot(page, 0)
  await page.mouse.dblclick(g.x(0), g.y(0.78))
  await expect(page.getByTestId('value-field')).toBeVisible()
  await page.getByTestId('value-input').fill('-12')
  await page.getByTestId('value-input').press('Enter')
  expect(pointsOf((await lanes(page))[0]![1])[0]!.value).toBe(-12)
  await page.keyboard.press('Control+z')
  expect(pointsOf((await lanes(page))[0]![1])[0]!.value).toBe(0)

  // A note held on the track: the master's level follows the lane when the transport passes the second point.
  await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x90, 69, 127]))
  await expect.poll(() => master(page)).toBeGreaterThan(0.05)
  const loud = await master(page)
  await page.getByTestId('transport-play').click()
  // One bar at 120 BPM is two seconds: before it the lane holds 0 dB, after it -60 dB.
  await expect.poll(() => master(page), { timeout: 15_000 }).toBeLessThan(loud / 100)
  await page.getByTestId('transport-stop').click()
  await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x80, 69, 0]))

  // Delete the lane by its head: one `automation.lane.delete`, and undo brings it back with its points.
  await page.getByTestId('arr-lane-delete').first().click()
  await expect(page.getByTestId('arr-lane')).toHaveCount(0)
  expect(await lanes(page)).toHaveLength(0)
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('arr-lane')).toHaveCount(1)
  expect(pointsOf((await lanes(page))[0]![1])).toHaveLength(2)

  expect(seen.errors).toEqual([])
})

test('automation: changing the instrument takes its lanes with it, says so, and undo gives them back', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await open(page)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByTestId('panel-devices').click()
  await addLane(page, 'Gain')
  const [laneId] = (await lanes(page))[0]!
  await run(page, { type: 'automation.points.add', lane: laneId, points: [{ tick: 0, value: 0.5, shape: 'linear' }] })
  await addLane(page, 'Volume')
  expect(await lanes(page)).toHaveLength(2)

  // Before the swap: the warning, with the count of lanes the instrument has.
  await expect(page.getByTestId('instrument-lanes-warning')).toContainText('(1)')
  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/instrument-warning.png` })
  await page.getByTestId('browse-instrument').filter({ hasText: 'Drums' }).click()
  // The instrument's lane is gone, the mixer's stays, and the page says what happened.
  await expect(page.getByTestId('instrument-lanes-removed')).toContainText('(1)')
  expect(await lanes(page)).toHaveLength(1)
  await expect(page.getByTestId('arr-lane')).toHaveCount(1)
  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/instrument-removed.png` })
  // One undo: the instrument and its lane come back together.
  await page.keyboard.press('Control+z')
  expect(await lanes(page)).toHaveLength(2)
  await expect(page.getByTestId('arr-lane')).toHaveCount(2)
  await expect(page.getByTestId('instrument-lanes-warning')).toBeVisible()
  expect(seen.errors).toEqual([])
})

test('automation: stays fluid with 10 000 points in sight', async ({ page }) => {
  test.setTimeout(240_000)
  const seen = watch(page)
  await open(page, 1800)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await addLane(page, 'Gain')
  const [laneId] = (await lanes(page))[0]!
  // 10 000 points over 100 bars, a sine-like value, a step every tenth point.
  const POINTS = 10_000
  await page.evaluate(
    ({ laneId, n, bar }) => {
      const run = (window as unknown as { __faulkRun: (c: unknown) => void }).__faulkRun
      const points = Array.from({ length: n }, (_, i) => ({
        tick: i * Math.round((100 * bar) / n),
        value: 0.5 + 0.45 * Math.sin(i / 17),
        shape: i % 10 === 0 ? 'step' : 'linear',
      }))
      run({ type: 'automation.points.add', lane: laneId, points })
    },
    { laneId, n: POINTS, bar: BAR },
  )
  await expect(page.getByTestId('arr-lane').first()).toHaveAttribute('data-points', String(POINTS))
  // Zoomed all the way out: the whole lane, more than one point a pixel.
  const grid = stage(page).getByTestId('timegrid')
  await page.evaluate(() => {
    const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
    const box = root.getBoundingClientRect()
    root.dispatchEvent(
      new WheelEvent('wheel', { bubbles: true, cancelable: true, ctrlKey: true, deltaY: 20000, clientX: box.left + 10, clientY: box.top + 100 }),
    )
  })
  await expect.poll(async () => Number(await grid.getAttribute('data-px-per-tick'))).toBeLessThan(0.01)
  await grid.press('Home')
  await expect.poll(async () => Number(await stage(page).getAttribute('data-lane-points'))).toBeGreaterThan(5000)
  expect(await stage(page).getAttribute('data-lane-dense')).toBe('true')
  if (process.env.FAULK_E2E_SHOTS) await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/lane-10000.png` })

  // The cost of the frames drawn while the view is swept (scroll and zoom), and while a drag of all the points is previewed.
  const sweep = () =>
    page.evaluate(
      () =>
        new Promise<{ frames: number; intervals: number[]; cost: number[] }>((done) => {
          const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
          const box = root.getBoundingClientRect()
          const intervals: number[] = []
          const cost: number[] = []
          let last = 0
          let lastDraw = Number(root.dataset.drawCount)
          let frame = 0
          const send = (init: WheelEventInit) =>
            root.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, clientX: box.left + 300, clientY: box.top + 200, ...init }))
          const step = (now: number) => {
            if (last) intervals.push(now - last)
            last = now
            const drawn = Number(root.dataset.drawCount)
            if (drawn !== lastDraw) cost.push(Number(root.dataset.drawMs) + Number(root.dataset.bodyMs))
            lastDraw = drawn
            if (frame < 240) {
              const n = frame++
              if (n < 120) send({ ctrlKey: true, deltaY: n % 30 < 15 ? -150 : 150 })
              else send({ shiftKey: true, deltaY: n < 180 ? 500 : -500 })
              requestAnimationFrame(step)
            } else done({ frames: frame, intervals, cost })
          }
          requestAnimationFrame(step)
        }),
    )
  const mean = (values: number[]) => values.reduce((a, b) => a + b, 0) / values.length
  const pct = (values: number[], p: number) => [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * p))]!
  const swept = await sweep()
  console.log(
    `timing: ${STRICT_TIMING ? 'STRICT' : 'ceiling'} lane with ${POINTS} points, scroll and zoom: ${swept.frames} frames, frame interval mean ${mean(swept.intervals).toFixed(2)} ms, p95 ${pct(swept.intervals, 0.95).toFixed(2)} ms, worst ${Math.max(...swept.intervals).toFixed(2)} ms; draw (${swept.cost.length} drawn frames, grid + clips + lane) mean ${mean(swept.cost).toFixed(3)} ms, p95 ${pct(swept.cost, 0.95).toFixed(3)} ms, worst ${Math.max(...swept.cost).toFixed(3)} ms`,
  )
  expect(swept.cost.length).toBeGreaterThan(60)
  if (STRICT_TIMING) expect(mean(swept.intervals)).toBeLessThan(17.5)
  expect(pct(swept.cost, 0.95)).toBeLessThan(STRICT_TIMING ? 4 : 12)

  // A drag of all the points at once, by the pointer: select all, grab one, move it; the frames drawn meanwhile.
  await grid.press('Home')
  await page.evaluate(() => {
    const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
    const box = root.getBoundingClientRect()
    root.dispatchEvent(
      new WheelEvent('wheel', { bubbles: true, cancelable: true, ctrlKey: true, deltaY: 20000, clientX: box.left + 10, clientY: box.top + 100 }),
    )
  })
  // The first point is at tick 0 and the height of its value (0.5): a click selects it, Ctrl+A selects the lane's.
  await page.mouse.click((await spot(page, 0)).x(0) + 1, (await spot(page, 0)).y(0.5))
  await page.keyboard.press('Control+a')
  await expect(page.getByTestId('arr-count')).toContainText('10000')
  const g = await spot(page, 0)
  // The first point is at tick 0, at its value's height.
  const first = pointsOf((await lanes(page))[0]![1])[0]!
  const ys = g.y(first.value)
  const { costs, moves } = await page.evaluate(
    ({ x, y, to }) =>
      new Promise<{ costs: number[]; moves: number[] }>((done) => {
        const root = document.querySelector('[data-testid=timegrid]') as HTMLElement
        const stage = document.querySelector('[data-testid=arr-stage]') as HTMLElement
        const fire = (type: string, cx: number, cy: number) =>
          stage.dispatchEvent(
            new PointerEvent(type, {
              bubbles: true,
              cancelable: true,
              pointerId: 1,
              button: 0,
              buttons: type === 'pointerup' ? 0 : 1,
              clientX: cx,
              clientY: cy,
            }),
          )
        const cost: number[] = []
        const moves: number[] = []
        let lastDraw = Number(root.dataset.drawCount)
        fire('pointerdown', x, y)
        let n = 0
        const step = () => {
          const drawn = Number(root.dataset.drawCount)
          if (drawn !== lastDraw) cost.push(Number(root.dataset.drawMs) + Number(root.dataset.bodyMs))
          lastDraw = drawn
          if (n < 90) {
            n++
            const t0 = performance.now()
            fire('pointermove', x + (to.x - x) * (n / 90), y + (to.y - y) * (n / 90))
            queueMicrotask(() => moves.push(performance.now() - t0))
            requestAnimationFrame(step)
          } else {
            fire('pointerup', to.x, to.y)
            done({ costs: cost, moves })
          }
        }
        requestAnimationFrame(step)
      }),
    { x: g.x(0) + 1, y: ys, to: { x: g.x(0) + 40, y: ys + 30 } },
  )
  console.log(
    `timing: ${STRICT_TIMING ? 'STRICT' : 'ceiling'} lane with ${POINTS} points, drag of all the points: ${costs.length} drawn frames, draw mean ${mean(costs).toFixed(3)} ms, p95 ${pct(costs, 0.95).toFixed(3)} ms, worst ${Math.max(...costs).toFixed(3)} ms; handling a move (handler, preview of the lane, effects) mean ${mean(moves).toFixed(3)} ms, p95 ${pct(moves, 0.95).toFixed(3)} ms, worst ${Math.max(...moves).toFixed(3)} ms`,
  )
  expect(costs.length).toBeGreaterThan(20)
  expect(pct(costs, 0.95)).toBeLessThan(STRICT_TIMING ? 4 : 12)
  expect(pct(moves, 0.95)).toBeLessThan(STRICT_TIMING ? 8 : 40)
  // The drag was ONE group: one undo gives the lane back.
  await page.keyboard.press('Control+z')
  expect(pointsOf((await lanes(page))[0]![1])).toHaveLength(POINTS)
  expect(pointsOf((await lanes(page))[0]![1])[0]!.tick).toBe(0)
  expect(seen.errors).toEqual([])
})
