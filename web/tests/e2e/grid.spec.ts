import { expect, test, type Page } from '@playwright/test'
import { best, mainThreadMeter, watch } from './support'

const STRICT = process.env.FAULK_E2E_STRICT_TIMING === '1'
/** Measured windows per performance check; the best one counts, because noise only adds time. */
const WINDOWS = 3
/**
 * Ceilings on the main-thread time per frame, in reference units (see `mainThreadMeter`): the sweep
 * costs about 1 (up to 2 with a 4x slower CPU) and a scene playing about 0.25 (up to 0.4); ten times
 * the cost (an order-of-magnitude regression) crosses both ceilings, a noisy runner does not.
 */
const SWEEP_CEILING = 4
const PLAYING_CEILING = 1.2

// GR-3 (#76): the launch grid in a real browser. Real worklet, engine and project core: scenes and
// clips are made by the grid's own gestures, notes are drawn in the piano roll, and the cell states
// and the output level come from the engine's report, so a launch is proven by what is heard.

const SILENCE_DB = -60
const ROW = 16
const RULER = 28

const startAudio = (page: Page) =>
  page
    .getByTestId('engine-state')
    .getByRole('button', { name: /Start audio|Démarrer/ })
    .click()
/** The output level of the engine's report, in dBFS (the top bar carries it as data). */
const meterDb = async (page: Page) => {
  const peak = Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))
  return peak > 0 ? 20 * Math.log10(peak) : -Infinity
}
const cells = (page: Page) => page.getByTestId('cell')
/** The cell of the first track at a scene (0 first). */
const cell = (page: Page, scene: number) => page.getByTestId('scene-row').nth(scene).getByTestId('cell').first()
const roll = (page: Page) => page.getByTestId('piano-roll')

async function addScene(page: Page) {
  const before = await page.getByTestId('scene-row').count()
  await page.getByTestId('add-scene').click()
  await expect(page.getByTestId('scene-row')).toHaveCount(before + 1)
}

/** Draws one note along the whole clip in the open roll (snap: bar), so the clip sounds all through. */
async function drawLongNote(page: Page) {
  await expect.poll(async () => Number(await page.getByTestId('timegrid').getAttribute('data-draw-count'))).toBeGreaterThan(0)
  await page.getByTestId('roll-snap').selectOption('bar')
  const box = (await page.getByTestId('roll-stage').boundingBox())!
  const ppt = Number(await page.getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollY = Number(await page.getByTestId('roll-stage').getAttribute('data-scroll-y'))
  const y = box.y + RULER + (127 - 60) * ROW - scrollY + ROW / 2
  await page.mouse.move(box.x + 5 * ppt, y)
  await page.mouse.down()
  await page.mouse.move(box.x + 1900 * ppt, y, { steps: 4 })
  await page.mouse.move(box.x + 3800 * ppt, y, { steps: 4 })
  await page.mouse.up()
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')
}

/** A clip in the cell of the scene, by a double-click; the roll opens on it and a note is drawn. */
async function makeClip(page: Page, scene: number) {
  await cell(page, scene).dblclick()
  await expect(cell(page, scene)).toHaveAttribute('data-clip', /.+/)
  await expect(roll(page)).toBeVisible()
  await drawLongNote(page)
}

test('launch grid: scenes and clips by gestures, launch is heard, states follow the engine, scene operations undo', async ({ page }) => {
  test.setTimeout(180_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await page.getByTestId('nav-grid').click()
  await expect(page.getByTestId('grid')).toBeVisible()
  await expect(page.getByTestId('grid-no-scenes')).toBeVisible()

  // Two scenes, a clip in each cell of the one track; the piano roll follows the double-click.
  await addScene(page)
  await addScene(page)
  await expect(cells(page)).toHaveCount(2)
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'empty')
  await makeClip(page, 0)
  await makeClip(page, 1)
  await expect(page.getByTestId('bottom-panel')).toHaveAttribute('data-panel', 'piano-roll')
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'idle')
  await expect(cell(page, 1)).toHaveAttribute('data-state', 'idle')
  // The audition of the notes just drawn dies away: nothing plays before a launch.
  await expect.poll(() => meterDb(page), { timeout: 8_000 }).toBeLessThan(SILENCE_DB)

  // Launching the clip of the first scene while stopped starts the transport; the clip plays and is heard.
  await cell(page, 0).click()
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'playing', { timeout: 10_000 })
  await expect(cell(page, 0).locator('svg')).toHaveAttribute('data-glyph', 'playing')
  await expect.poll(() => meterDb(page), { timeout: 5_000 }).toBeGreaterThan(SILENCE_DB)
  await expect(page.getByTestId('feedback')).toHaveAttribute('data-transport', 'playing')
  // A clip made by the grid loops: past its first bar it still plays.
  await expect.poll(async () => Number(await page.getByTestId('feedback').getAttribute('data-tick')), { timeout: 8_000 }).toBeGreaterThan(4000)
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'playing')
  expect(await meterDb(page)).toBeGreaterThan(SILENCE_DB)

  // The scene button of the second scene takes the track over at the boundary: the first clip stops.
  await page.getByTestId('scene-row').nth(1).getByTestId('launch-scene').click()
  await expect(cell(page, 1)).toHaveAttribute('data-state', /queued|playing/)
  await expect(cell(page, 1)).toHaveAttribute('data-state', 'playing', { timeout: 10_000 })
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'idle')

  // Stopping the track: the clip stops at its boundary and the output falls silent.
  await page.getByTestId('stop-track').click()
  await expect(cell(page, 1)).toHaveAttribute('data-state', 'idle', { timeout: 10_000 })
  await expect.poll(() => meterDb(page), { timeout: 8_000 }).toBeLessThan(SILENCE_DB)

  // Stop all stops a launched scene too.
  await page.getByTestId('scene-row').nth(0).getByTestId('launch-scene').click()
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'playing', { timeout: 10_000 })
  await page.getByTestId('stop-all').click()
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'idle', { timeout: 10_000 })
  await expect.poll(() => meterDb(page), { timeout: 8_000 }).toBeLessThan(SILENCE_DB)

  // Scenes: rename (double-click, Enter), then undo and redo.
  const names = page.getByTestId('scene-name')
  await names.nth(0).dblclick()
  await page.getByRole('textbox', { name: /Scene name|Nom de la scène/ }).fill('Verse')
  await page.keyboard.press('Enter')
  await expect(names.nth(0)).toHaveText('Verse')
  await page.keyboard.press('Control+z')
  await expect(names.nth(0)).toHaveText('Scene 1')
  await page.keyboard.press('Control+y')
  await expect(names.nth(0)).toHaveText('Verse')

  // Move the second scene up through the menu, then undo.
  const order = () => page.getByTestId('scene-row').evaluateAll((rows) => rows.map((row) => row.getAttribute('data-scene')))
  const [first, second] = await order()
  await page.getByTestId('scene-more').nth(1).click()
  await page.getByRole('menuitem', { name: 'Move up' }).click()
  await expect.poll(order).toEqual([second, first])
  await page.keyboard.press('Control+z')
  await expect.poll(order).toEqual([first, second])
  await expect(names.nth(0)).toHaveText('Verse')

  // Deleting a scene with a clip asks first; cancelling changes nothing, deleting is one undo from coming back.
  await page.getByTestId('scene-more').nth(0).click()
  await page.getByTestId('scene-delete').click()
  await expect(page.getByTestId('scene-confirm')).toContainText('Delete Verse and its clip? You can undo this.')
  await page.getByRole('button', { name: 'Cancel' }).click()
  await expect(page.getByTestId('scene-row')).toHaveCount(2)
  await page.getByTestId('scene-more').nth(0).click()
  await page.getByTestId('scene-delete').click()
  await page.getByTestId('scene-confirm-delete').click()
  await expect(page.getByTestId('scene-row')).toHaveCount(1)
  await page.keyboard.press('Control+z')
  await expect(page.getByTestId('scene-row')).toHaveCount(2)
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'idle')
  await expect(names.nth(0)).toHaveText('Verse')

  // The clip still plays after the undo (the engine got the scene and its clip back).
  await cell(page, 0).click()
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'playing', { timeout: 10_000 })
  await expect.poll(() => meterDb(page), { timeout: 5_000 }).toBeGreaterThan(SILENCE_DB)

  // An empty scene goes at once, with no question.
  await addScene(page)
  await page.getByTestId('scene-more').nth(2).click()
  await page.getByTestId('scene-delete').click()
  await expect(page.getByTestId('scene-row')).toHaveCount(2)

  // The edit button opens a clip in the roll without launching it.
  await cell(page, 1).hover()
  await page.getByTestId('edit-clip').nth(1).click()
  await expect(roll(page)).toHaveAttribute('data-note-count', '1')

  expect(seen.errors).toEqual([])
})

// 32 tracks x 64 scenes with a clip in every cell (2 048 clips), the worst case for the grid: the cost
// of showing it, of a frame while scrolling it in both directions, and of the engine's states reaching
// it while a scene plays. The page's dev hook (`__faulkRun`, `__faulkView`) builds the project.
test('launch grid: stays fluid with 32 tracks and 64 scenes @full', async ({ page }) => {
  test.setTimeout(240_000)
  const seen = watch(page)
  await page.setViewportSize({ width: 1280, height: 800 })
  await page.goto('/app')
  await startAudio(page)
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  const hook = await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
  test.skip(hook !== 'function', 'the dispatcher hook exists in the dev server only')

  const built = await page.evaluate(() => {
    const w = window as unknown as {
      __faulkRun: (c: unknown) => void
      __faulkView: () => { project: { tracks: Record<string, unknown>; scenes: Record<string, unknown> } }
    }
    const t0 = performance.now()
    for (let i = 0; i < 31; i++) w.__faulkRun({ type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth' })
    for (let i = 0; i < 64; i++) w.__faulkRun({ type: 'scene.create' })
    const { tracks, scenes } = w.__faulkView().project
    for (const track of Object.keys(tracks)) for (const scene of Object.keys(scenes)) w.__faulkRun({ type: 'clip.create', track, at: { at: 'slot', scene } })
    const clips = Object.values(w.__faulkView().project.tracks as Record<string, { clips: object }>).reduce((n, t) => n + Object.keys(t.clips).length, 0)
    return { ms: performance.now() - t0, tracks: Object.keys(tracks).length, scenes: Object.keys(scenes).length, clips }
  })
  expect(built).toMatchObject({ tracks: 32, scenes: 64, clips: 2048 })

  // The grid view mounts: from the click to the first frame after it.
  const mount = await page.evaluate(
    () =>
      new Promise<number>((done) => {
        const start = performance.now()
        ;(document.querySelector('[data-testid=nav-grid]') as HTMLElement).click()
        requestAnimationFrame(() => requestAnimationFrame(() => done(performance.now() - start)))
      }),
  )
  await expect(page.getByTestId('grid')).toBeVisible()
  const drawn = await cells(page).count()

  // Sweep: scroll down, back up, and across, one step a frame; the frame intervals are the cost.
  const sweep = () =>
    page.evaluate(
      () =>
        new Promise<{ intervals: number[]; rows: number; columns: number }>((done) => {
          const el = document.querySelector('.scroll') as HTMLElement
          const intervals: number[] = []
          const rows = new Set<string>()
          const columns = new Set<string>()
          let last = 0
          let frame = 0
          const FRAMES = 240
          const step = (now: number) => {
            if (last) intervals.push(now - last)
            last = now
            for (const cell of document.querySelectorAll<HTMLElement>('[data-testid=cell]')) {
              rows.add(cell.dataset.si!)
              columns.add(cell.dataset.track!)
            }
            if (frame < FRAMES) {
              const down = frame < 80 ? frame : frame < 160 ? 160 - frame : 0
              el.scrollTop = down * 40
              el.scrollLeft = frame < 160 ? 0 : (frame - 160) * 30
              frame++
              requestAnimationFrame(step)
            } else done({ intervals, rows: rows.size, columns: columns.size })
          }
          requestAnimationFrame(step)
        }),
    )
  const sorted = (values: number[]) => [...values].sort((a, b) => a - b)
  const pct = (values: number[], p: number) => sorted(values)[Math.min(values.length - 1, Math.floor(values.length * p))]!
  const mean = (values: number[]) => values.reduce((a, b) => a + b, 0) / values.length
  // The cost is the main-thread time per frame (see `mainThreadMeter`), the best of three sweeps: the
  // frame intervals depend on the runner (vsync, neighbours) and are printed, not asserted.
  const meter = await mainThreadMeter(page)
  const sweeps = []
  for (let i = 0; i < WINDOWS; i++) sweeps.push(await meter.measure(sweep))
  const swept = sweeps[0]!.result
  const sweptCost = best(sweeps.map((s) => s.cost.task / s.result.intervals.length / s.cost.unit))
  const mode = STRICT ? 'STRICT' : 'ceiling'
  console.log(
    `timing: ${mode} launch grid 32 x 64: project built in ${built.ms.toFixed(0)} ms, view mounted in ${mount.toFixed(0)} ms with ${drawn} cells drawn; scroll sweep ${swept.intervals.length} frames, frame interval mean ${mean(swept.intervals).toFixed(2)} ms, p95 ${pct(swept.intervals, 0.95).toFixed(2)} ms, worst ${Math.max(...swept.intervals).toFixed(2)} ms; main thread per frame, best of ${WINDOWS}: ${sweptCost.toFixed(3)} reference units (reference ${sweeps[0]!.cost.unit.toFixed(1)} ms)`,
  )
  // The sweep went over the whole grid, drawing only a part of it at a time; its two ends are reachable.
  expect(swept.rows).toBe(64)
  expect(swept.columns).toBeGreaterThan(20)
  expect(drawn).toBeLessThan(400)
  await page.evaluate(() => Object.assign(document.querySelector('.scroll') as HTMLElement, { scrollTop: 1e6, scrollLeft: 1e6 }))
  await expect(page.locator('[data-testid=cell][data-si="63"][data-ti="31"]')).toBeVisible()
  expect(mount).toBeLessThan(STRICT ? 250 : 750)
  expect(sweptCost).toBeLessThan(STRICT ? 2 : SWEEP_CEILING)

  // A scene plays: the states reach the cells while the grid is scrolled, and the frames stay short.
  await page.evaluate(() => Object.assign(document.querySelector('.scroll') as HTMLElement, { scrollTop: 0, scrollLeft: 0 }))
  await page.getByTestId('scene-row').first().getByTestId('launch-scene').click()
  await expect(cells(page).first()).toHaveAttribute('data-state', 'playing', { timeout: 10_000 })
  const play = () =>
    page.evaluate(
      () =>
        new Promise<number[]>((done) => {
          const intervals: number[] = []
          let last = 0
          const step = (now: number) => {
            if (last) intervals.push(now - last)
            last = now
            if (intervals.length < 120) requestAnimationFrame(step)
            else done(intervals)
          }
          requestAnimationFrame(step)
        }),
    )
  const plays = []
  for (let i = 0; i < WINDOWS; i++) plays.push(await meter.measure(play))
  const playing = plays[0]!.result
  const playingCost = best(plays.map((p) => p.cost.task / (p.result.length + 1) / p.cost.unit))
  console.log(
    `timing: ${mode} launch grid 32 x 64, a scene playing: ${playing.length} frames, frame interval mean ${mean(playing).toFixed(2)} ms, p95 ${pct(playing, 0.95).toFixed(2)} ms, worst ${Math.max(...playing).toFixed(2)} ms; main thread per frame, best of ${WINDOWS}: ${playingCost.toFixed(3)} reference units`,
  )
  expect(playingCost).toBeLessThan(STRICT ? 0.5 : PLAYING_CEILING)
  expect(seen.errors).toEqual([])
})

test('launch grid: the keyboard reaches the cells and creates a clip', async ({ page }) => {
  const seen = watch(page)
  await page.goto('/app')
  await startAudio(page)
  await page.getByTestId('nav-grid').click()
  await addScene(page)
  await addScene(page)
  // One cell is in the tab order; the arrows move between cells, Enter on an empty cell creates its clip.
  await cell(page, 0).focus()
  await page.keyboard.press('ArrowDown')
  await expect(cell(page, 1)).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(cell(page, 1)).toHaveAttribute('data-clip', /.+/)
  await expect(cell(page, 0)).toHaveAttribute('data-state', 'empty')
  // The clip loops over its whole length; growing it keeps the loop whole, and one undo takes the growth back.
  await expect(roll(page)).toHaveAttribute('data-loop', '0-3840')
  await page.getByTestId('roll-longer').click()
  await expect(roll(page)).toHaveAttribute('data-clip-length', '7680')
  await expect(roll(page)).toHaveAttribute('data-loop', '0-7680')
  await page.keyboard.press('Control+z')
  await expect(roll(page)).toHaveAttribute('data-clip-length', '3840')
  await expect(roll(page)).toHaveAttribute('data-loop', '0-3840')
  // Creating the clip and its loop is one history entry: one undo empties the cell.
  await page.keyboard.press('Control+z')
  await expect(cell(page, 1)).toHaveAttribute('data-state', 'empty')
  expect(seen.errors).toEqual([])
})
