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

async function open(page: Page, width = 1600, midi = false, lang = 'en') {
  await page.setViewportSize({ width, height: 900 })
  if (lang !== 'en') await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
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

  // Stopped at the start, the Gain knob shows the lane's value there (what sounds at Play) and says that a lane drives it (#388);
  // a seek while stopped moves it, and a knob that no lane drives has no mark.
  const gain = page.getByTestId('device-card').first().getByTestId('param-gain').getByRole('slider')
  await expect(gain).toHaveAttribute('data-automated', 'true')
  await expect(gain).toHaveAttribute('title', /Oscillator › Gain/)
  await expect(page.getByTestId('device-card').first().getByTestId('param-attack').getByRole('slider')).not.toHaveAttribute('data-automated', /.*/)
  const stored = await gain.getAttribute('aria-valuetext')
  await run(page, { type: 'transport.seek', tick: BAR / 2 })
  await expect(gain).not.toHaveAttribute('aria-valuetext', stored!)
  await run(page, { type: 'transport.seek', tick: 0 })
  await expect(gain).toHaveAttribute('aria-valuetext', stored!)
  // Play: the Gain knob of the device panel moves with the lane (the engine plays the lane; the panel shows it).
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
  // Stopped (the playhead goes back to the start), the control shows the lane's value there again.
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

  // A value past the range is limited and the editor says so (#421); an unreadable one changes nothing and the field says so.
  await page.mouse.dblclick(g.x(0), g.y(0.78))
  await page.getByTestId('value-input').fill('20')
  await page.getByTestId('value-input').press('Enter')
  await expect(page.getByTestId('value-note')).toHaveText('Limited to 6.0 dB')
  expect(pointsOf((await lanes(page))[0]![1])[0]!.value).toBe(6)
  await page.keyboard.press('Control+z')
  await stage(page).getByTestId('timegrid').focus()
  await page.mouse.dblclick(g.x(0), g.y(0.78))
  await page.getByTestId('value-input').fill('-200')
  await page.getByTestId('value-input').press('Enter')
  await expect(page.getByTestId('value-note')).toHaveText('Limited to −∞ dB')
  await page.keyboard.press('Control+z')
  await stage(page).getByTestId('timegrid').focus()
  await page.mouse.dblclick(g.x(0), g.y(0.78))
  await page.getByTestId('value-input').fill('loud')
  await page.getByTestId('value-input').press('Enter')
  await expect(page.getByTestId('value-error')).toHaveText('Value not recognised')
  expect(pointsOf((await lanes(page))[0]![1])[0]!.value).toBe(0)
  await page.getByTestId('value-input').press('Escape')
  await expect(page.getByTestId('value-note')).toHaveCount(0, { timeout: 5000 })

  // Stopped, the track list and the mixer show the lane's value at the playhead, marked as driven by a lane, and follow a seek (#388).
  const listGain = page.getByTestId('track-header').first().getByTestId('track-gain-readout')
  const listFader = page.getByTestId('track-header').first().getByRole('slider').first()
  await expect(listGain).toHaveText('0.0')
  await expect(listFader).toHaveAttribute('data-automated', 'true')
  await expect(listFader).toHaveAttribute('title', /Mixer › Volume/)
  await run(page, { type: 'transport.seek', tick: BAR })
  await expect(listGain).toHaveText('−60.0')
  await page.getByTestId('nav-mixer').click()
  await expect(page.getByTestId('mixer-gain').first()).toHaveText('−60.0 dB')
  await expect(page.getByTestId('mixer-fader').first()).toHaveAttribute('data-automated', 'true')
  await expect(page.getByTestId('mixer-pan').first()).not.toHaveAttribute('data-automated', /.*/)
  // A fader gesture while stopped holds until the next seek (AM-2): the mixer shows it, the seek gives the lane back.
  await run(page, { type: 'mixer.set_volume', track: (await tracks(page))[0]![0], gain_db: -12 })
  await expect(page.getByTestId('mixer-gain').first()).toHaveText('−12.0 dB')
  await run(page, { type: 'transport.seek', tick: 0 })
  await expect(page.getByTestId('mixer-gain').first()).toHaveText('0.0 dB')
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toBeVisible()
  const toggle = page.getByTestId('arr-automation-toggle').first()
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click()
  await expect(page.getByTestId('arr-lane')).toHaveCount(1)

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

test('automation: a lane of the master (#378) is shown on the master fader and played by the engine', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await fakeWebMidi(page)
  await open(page, 1600, true)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  // A lane on the master's volume, by the command an agent would use: 0 dB for a bar, then -60 dB.
  await run(page, { type: 'automation.lane.create', master: true, target: { mixer: { param: 'gain' } } })
  const masterLanes = () =>
    page.evaluate(() =>
      Object.keys((window as unknown as { __faulkView(): { project: { master_automation?: object } } }).__faulkView().project.master_automation ?? {}),
    )
  await expect.poll(async () => (await masterLanes()).length).toBe(1)
  const [laneId] = await masterLanes()
  await run(page, {
    type: 'automation.points.add',
    lane: laneId,
    points: [
      { tick: 0, value: 0, shape: 'step' },
      { tick: BAR, value: -60, shape: 'step' },
    ],
  })
  // The master strip of the mixer shows the lane's value at the playhead, marked as driven, and follows a seek.
  await page.getByTestId('nav-mixer').click()
  const strip = page.getByTestId('mixer-master')
  await expect(strip.getByTestId('mixer-gain')).toHaveText('0.0 dB')
  await expect(page.getByTestId('mixer-master-fader')).toHaveAttribute('data-automated', 'true')
  await run(page, { type: 'transport.seek', tick: BAR })
  await expect(strip.getByTestId('mixer-gain')).toHaveText('−60.0 dB')
  // A track's fader is not driven by the master's lane.
  await expect(page.getByTestId('mixer-fader').first()).not.toHaveAttribute('data-automated', /.*/)
  await run(page, { type: 'transport.seek', tick: 0 })
  await expect(strip.getByTestId('mixer-gain')).toHaveText('0.0 dB')

  // The engine really plays it: a note held on the track, the master's level falls at the second point (one bar is two seconds).
  await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x90, 69, 127]))
  await expect.poll(() => master(page)).toBeGreaterThan(0.05)
  const loud = await master(page)
  await page.getByTestId('transport-play').click()
  await expect.poll(() => master(page), { timeout: 15_000 }).toBeLessThan(loud / 100)
  await page.getByTestId('transport-stop').click()
  await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x80, 69, 0]))

  // One undo takes the points back, another the lane.
  await page.keyboard.press('Control+z')
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await masterLanes()).length).toBe(0)
  expect(seen.errors).toEqual([])
})

test('automation: a limited or unreadable typed value says so in French (#421)', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await fakeWebMidi(page)
  await open(page, 1600, true, 'fr')
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByTestId('arr-snap').selectOption('beat')
  await addLane(page, 'Volume')
  const [laneId] = (await lanes(page))[0]!
  await run(page, { type: 'automation.points.add', lane: laneId, points: [{ tick: 0, value: 0, shape: 'step' }] })
  await expect(page.getByTestId('arr-lane').first()).toHaveAttribute('data-points', '1')
  await stage(page).getByTestId('timegrid').focus()
  const g = await spot(page, 0)
  await page.mouse.dblclick(g.x(0), g.y(0.78))
  await page.getByTestId('value-input').fill('20')
  await page.getByTestId('value-input').press('Enter')
  await expect(page.getByTestId('value-note')).toHaveText('Limité à 6.0 dB')
  await page.keyboard.press('Control+z')
  await stage(page).getByTestId('timegrid').focus()
  // The toolbar can wrap differently once the selection count changes, which moves the stage: measure again.
  const g2 = await spot(page, 0)
  await page.mouse.dblclick(g2.x(0), g2.y(0.78))
  await page.getByTestId('value-input').fill('fort')
  await page.getByTestId('value-input').press('Enter')
  await expect(page.getByTestId('value-error')).toHaveText('Valeur non reconnue')
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
  // The same swap by an agent (a command that does not come from the browser's buttons): the page says so too.
  const track = (await tracks(page))[0]![0]
  await run(page, { type: 'track.set_instrument', track, instrument: 'faulk.drums' })
  await expect(page.getByTestId('instrument-lanes-removed')).toContainText('(1)')
  await expect(page.getByTestId('arr-lane')).toHaveCount(1)
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
  // All the points of the lane are in sight (the count of the points the last frame looked at).
  await expect.poll(async () => Number(await stage(page).getAttribute('data-lane-points'))).toBe(POINTS)
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

test('automation: master row in the timeline, a fade drawn, level confirmed', async ({ page }) => {
  // #407: the master row is pinned at the bottom of the arrangement timeline; its lanes are added
  // from the arrangement's add row or from the mixer's "Automate" button; a drawn lane is played
  // by the engine (the master's level falls at the second point, one bar in).
  test.setTimeout(120_000)
  const seen = watch(page)
  await fakeWebMidi(page)
  await open(page, 1600, true)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByTestId('arr-snap').selectOption('beat')

  // The master row is always visible at the bottom of the arrangement, below all tracks.
  const masterRow = page.getByTestId('arr-master-row')
  await expect(masterRow).toBeVisible()

  // The master's fold toggle is there but its lanes are collapsed by default.
  const masterToggle = masterRow.getByTestId('arr-automation-toggle')
  await expect(masterToggle).toHaveAttribute('aria-expanded', 'false')
  await masterToggle.click()
  await expect(masterToggle).toHaveAttribute('aria-expanded', 'true')

  // Add a Volume lane from the master's add-lane row.
  const masterAddLane = page.getByTestId('arr-add-lane').last()
  await masterAddLane.selectOption({ label: 'Volume' })
  // The master's lane appears.
  const masterLanes = () =>
    page.evaluate(() =>
      Object.keys((window as unknown as { __faulkView(): { project: { master_automation?: object } } }).__faulkView().project.master_automation ?? {}),
    )
  await expect.poll(async () => (await masterLanes()).length).toBe(1)
  await expect(page.getByTestId('arr-lane').last()).toHaveAttribute('data-target', 'Mixer › Volume')

  // Helper functions for reading master lane data.
  const masterLaneData = () =>
    page.evaluate(() => {
      const view = (window as unknown as { __faulkView(): { project: { master_automation?: Record<string, Lane> } } }).__faulkView()
      return Object.values(view.project.master_automation ?? {})
    }) as Promise<Lane[]>
  async function masterLanePoints() {
    return (await masterLaneData()).map((l) => l?.points ?? {})
  }

  // Draw the fade by pointer only: the master's level played at the end of this test is the fade drawn here,
  // not one added by a command. Click near the top at tick 0 (loud) and near the bottom at BAR (near silence,
  // the gain axis bottom is MIN_GAIN_DB = -96 dB).
  await stage(page).getByTestId('timegrid').focus()
  const masterLaneIndex = (await page.getByTestId('arr-lane').count()) - 1
  const topSpot = await spot(page, masterLaneIndex)
  await page.mouse.click(topSpot.x(0) + 1, topSpot.y(0.98))
  await expect.poll(async () => Object.keys((await masterLanePoints())[0] ?? {}).length).toBe(1)
  const lowSpot = await spot(page, masterLaneIndex)
  await page.mouse.click(lowSpot.x(BAR) - 1, lowSpot.y(0.02))
  await expect.poll(async () => Object.keys((await masterLanePoints())[0] ?? {}).length).toBe(2)

  // Move the second point by pointer to exercise the point-move path on the master lane, then poll for the new tick.
  // The snap is 'beat', so moving to BAR/2 (a beat boundary) and back to BAR lands on exact ticks.
  const ticksBefore = Object.keys((await masterLanePoints())[0] ?? {}).sort()
  const moveSpot = await spot(page, masterLaneIndex)
  await page.mouse.move(moveSpot.x(BAR), moveSpot.y(0.02))
  await page.mouse.down()
  await page.mouse.move(moveSpot.x(BAR / 2), moveSpot.y(0.02), { steps: 5 })
  await page.mouse.up()
  await expect.poll(async () => Object.keys((await masterLanePoints())[0] ?? {}).sort()).not.toEqual(ticksBefore)
  // Move it back to BAR so the fade reaches its bottom exactly at one bar for the level check below.
  const backSpot = await spot(page, masterLaneIndex)
  await page.mouse.move(backSpot.x(BAR / 2), backSpot.y(0.02))
  await page.mouse.down()
  await page.mouse.move(backSpot.x(BAR), backSpot.y(0.02), { steps: 5 })
  await page.mouse.up()
  await expect.poll(async () => (await masterLaneData())[0]?.points[BAR]?.value).toBeLessThan(-40)

  // A step curve set from the toolbar makes the drop sharp at BAR: select all points, then click the step button.
  await page.keyboard.press('Control+a')
  await expect(page.getByTestId('arr-curve-step')).toBeEnabled()
  await page.getByTestId('arr-curve-step').click()
  await expect.poll(async () => Object.values((await masterLaneData())[0]?.points ?? {}).every((p) => p.shape === 'step')).toBe(true)
  await expect(page.getByTestId('arr-lane').last()).toHaveAttribute('data-points', '2')

  // Delete the points by keyboard, then undo: they come back (the point-delete path on a master lane).
  // The step-curve toolbar click moved the focus off the grid; a box drag over the master lane selects
  // both points, then the grid is refocused so Delete reaches its handler.
  const boxSpot = await spot(page, masterLaneIndex)
  await page.mouse.move(boxSpot.x(0) - 10, boxSpot.y(1) - 6)
  await page.mouse.down()
  await page.mouse.move(boxSpot.x(BAR) + 10, boxSpot.y(0) + 6, { steps: 6 })
  await page.mouse.up()
  await stage(page).getByTestId('timegrid').focus()
  await page.keyboard.press('Delete')
  await expect.poll(async () => Object.keys((await masterLanePoints())[0] ?? {}).length).toBe(0)
  await page.keyboard.press('Control+z')
  await expect.poll(async () => Object.keys((await masterLanePoints())[0] ?? {}).length).toBe(2)

  // Delete the whole lane using the lane-delete button, then undo to bring it back with its 2 points.
  await page.getByTestId('arr-lane-delete').last().click()
  await expect.poll(async () => (await masterLanes()).length).toBe(0)
  await page.keyboard.press('Control+z')
  await expect.poll(async () => (await masterLanes()).length).toBe(1)
  await expect(page.getByTestId('arr-lane').last()).toHaveAttribute('data-points', '2')

  // The mixer's "Automate" button navigates to the arrangement AND opens the master lanes.
  await page.getByTestId('nav-mixer').click()
  const automateBtn = page.getByTestId('mixer-master-automate-gain')
  await expect(automateBtn).toBeVisible()
  // Close the master lanes first, then click Automate: it must reopen them.
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toBeVisible()
  if ((await masterToggle.getAttribute('aria-expanded')) === 'true') await masterToggle.click()
  await expect(masterToggle).toHaveAttribute('aria-expanded', 'false')
  await page.getByTestId('nav-mixer').click()
  await automateBtn.click()
  // We are now in the arrangement view and the master lanes are open.
  await expect(arr(page)).toBeVisible()
  await expect(masterToggle).toHaveAttribute('aria-expanded', 'true')

  // The "Automate" button on a master effect knob creates its lane and navigates to the arrangement.
  // Set up: add a compressor to the master chain.
  await page.getByTestId('nav-mixer').click()
  await page.getByTestId('mixer-master-devices').click()
  const devPanel = page.getByTestId('bottom-panel')
  await expect(devPanel).toHaveAttribute('data-panel', 'devices')
  await devPanel.getByRole('button', { name: /Add Compressor/ }).click()
  await expect(devPanel.getByTestId('device-card')).toHaveCount(1)
  // Close the master lanes, then click the Automate button on the first knob of the Compressor.
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toBeVisible()
  if ((await masterToggle.getAttribute('aria-expanded')) === 'true') await masterToggle.click()
  await expect(masterToggle).toHaveAttribute('aria-expanded', 'false')
  // The Automate button is visible in the devices panel (bottom panel).
  const automateKnob = devPanel.getByTestId('device-param-automate').first()
  await expect(automateKnob).toBeVisible()
  await automateKnob.click()
  // Clicking opens the arrangement view and unfolds the master lanes.
  await expect(arr(page)).toBeVisible()
  await expect(masterToggle).toHaveAttribute('aria-expanded', 'true')
  // A new master lane exists for the device parameter.
  const masterLanesAfterDevice = () =>
    page.evaluate(() => {
      const v = (window as unknown as { __faulkView(): { project: { master_automation?: object } } }).__faulkView()
      return Object.keys(v.project.master_automation ?? {})
    })
  await expect.poll(async () => (await masterLanesAfterDevice()).length).toBeGreaterThanOrEqual(2)
  // Clicking again is idempotent: no new lane created (the guard prevents it).
  await page.getByTestId('nav-mixer').click()
  await page.getByTestId('mixer-master-devices').click()
  await expect(devPanel.getByTestId('device-param-automate').first()).toBeVisible()
  await devPanel.getByTestId('device-param-automate').first().click()
  await expect.poll(async () => (await masterLanesAfterDevice()).length).toBe(2)

  // The engine plays the fade drawn above: a note held on the track, the master's level is loud before BAR and near silence after.
  await page.getByTestId('nav-arrangement').click()
  await expect(arr(page)).toBeVisible()
  await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x90, 69, 127]))
  await expect.poll(() => master(page)).toBeGreaterThan(0.05)
  const loud = await master(page)
  await page.getByTestId('transport-play').click()
  // Before BAR (first second at 120 BPM): the drawn lane's top point holds a loud level, so it must stay loud.
  await page.waitForTimeout(500)
  expect(await master(page)).toBeGreaterThan(loud / 10)
  // After BAR: the step point drops to the gain axis bottom (near -96 dB), so the level must fall far below loud.
  await expect.poll(() => master(page), { timeout: 15_000 }).toBeLessThan(loud / 100)
  await page.getByTestId('transport-stop').click()
  await page.evaluate(() => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send([0x80, 69, 0]))

  expect(seen.errors).toEqual([])
})

test('automation: the master row is visible in an empty project, above the welcome message', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await open(page)
  // With a track, the master row is there and the welcome message is not.
  await expect(page.getByTestId('arr-master-row')).toBeVisible()
  await expect(page.getByTestId('arrangement-empty')).toHaveCount(0)
  // Delete every track: the project is empty, the welcome message shows, and it no longer hides the master row.
  while ((await page.getByTestId('track-header').count()) > 0) {
    const header = page.getByTestId('track-header').first()
    await header.getByTestId('track-more').click()
    await header.getByTestId('track-delete').click()
  }
  const welcome = page.getByTestId('arrangement-empty')
  const masterRow = page.getByTestId('arr-master-row')
  await expect(welcome).toBeVisible()
  await expect(masterRow).toBeVisible()
  await expect(masterRow.getByTestId('arr-automation-toggle')).toHaveAttribute('aria-expanded', 'false')
  // The message sits in the tracks area, below the master row, and does not overlap it.
  const masterBox = (await masterRow.boundingBox())!
  const welcomeBox = (await welcome.boundingBox())!
  expect(welcomeBox.y).toBeGreaterThanOrEqual(masterBox.y + masterBox.height - 1)
  // The master row works with no track: its lane menu adds a lane, and the message stays.
  await masterRow.getByTestId('arr-automation-toggle').click()
  await page.getByTestId('arr-add-lane').last().selectOption({ label: 'Volume' })
  await expect(page.getByTestId('arr-lane')).toHaveCount(1)
  await expect(welcome).toBeVisible()
  // The message stays inside the stage once the master is unfolded, and the ruler stays above the master row.
  const stageBox = (await stage(page).boundingBox())!
  expect((await welcome.boundingBox())!.y).toBeLessThan(stageBox.y + stageBox.height)
  expect((await masterRow.boundingBox())!.y).toBeGreaterThanOrEqual(stageBox.y + 28)
  expect(seen.errors).toEqual([])
})

test('automation: a fader gesture equal to the stored value holds the lane in the engine and on the page (#417)', async ({ page }) => {
  test.setTimeout(120_000)
  const seen = watch(page)
  await fakeWebMidi(page)
  await open(page, 1600, true)
  test.skip((await hooks(page)) !== 'function', 'the dispatcher hook exists in the dev server only')
  const midi = (bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)
  // A note held at the track's fader: the level the engine reports with no lane.
  await midi([0x90, 69, 127])
  await expect.poll(() => master(page)).toBeGreaterThan(0.05)
  const loud = await master(page)
  const [track, strip] = (await tracks(page))[0]!
  const stored = (strip as unknown as { mixer: { gain_db: number | null } }).mixer.gain_db
  // A volume lane at -60 dB: stopped, the engine applies it at the playhead (#388).
  await addLane(page, 'Volume')
  const [laneId] = (await lanes(page))[0]!
  await run(page, { type: 'automation.points.add', lane: laneId, points: [{ tick: 0, value: -60, shape: 'step' }] })
  await expect(page.getByTestId('arr-lane').first()).toHaveAttribute('data-points', '1')
  await expect.poll(() => master(page)).toBeLessThan(loud / 100)
  const listGain = page.getByTestId('track-header').first().getByTestId('track-gain-readout')
  await expect(listGain).toHaveText('−60.0')

  // The same volume as the project holds: the command changes nothing, yet the lane gives way, in the engine and on the page.
  await run(page, { type: 'mixer.set_volume', track, gain_db: stored })
  await expect.poll(() => master(page)).toBeGreaterThan(loud / 2)
  await expect(listGain).toHaveText('0.0')
  // A seek gives the lane the hand back.
  await run(page, { type: 'transport.seek', tick: 0 })
  await expect.poll(() => master(page)).toBeLessThan(loud / 100)
  await expect(listGain).toHaveText('−60.0')
  // A mute that repeats the stored one holds nothing: the lane keeps playing.
  await run(page, { type: 'mixer.set_mute', track, mute: false })
  await expect(listGain).toHaveText('−60.0')
  await expect.poll(() => master(page)).toBeLessThan(loud / 100)
  await midi([0x80, 69, 0])
  expect(seen.errors).toEqual([])
})
