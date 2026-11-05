import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// AU-6 (#115): audio clips in the grid and the timeline, in a real browser with the real worklet, engine and project
// core. A reference WAV is imported into an audio track, put in the grid and launched, and `mixer.levels` (the meters an
// agent reads, #683) hears it; it is placed on the timeline from the Clips panel and heard there too; it is drawn with its
// waveform; the mouse moves, resizes and splits it, each gesture one history entry; a file dropped on the track's lane is
// imported and placed there; its gain is typed in the Clips panel; after a reload the placements are found again, with
// their waveforms drawn.

test.describe.configure({ timeout: 240_000 })

const FIXTURES = fileURLToPath(new URL('../../../crates/audio-import/tests/fixtures/', import.meta.url))
const RULER = 28
/** The height of a track's row: the `lane` token (#471). */
const LANE = 44
const BAR = 3840
/** The reference file: 22 050 frames at 44.1 kHz, half a second, 960 ticks at 120 bpm. */
const FILE_TICKS = 960

type Span = { frames?: { length: number; offset: number }; ticks?: { length: number; offset: number } }
type View = {
  project: {
    tracks: Record<
      string,
      {
        kind: string
        clips: Record<string, { slot: string | null; content: { audio?: { gain_db: number | null } } }>
        placements: Record<string, { clip: string; start: number; span: Span }>
      }
    >
  }
}
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun(c), command)
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
/** How many entries the history can undo: a gesture adds one. */
const historyCount = (page: Page) => page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: unknown[] } }).__faulkHistory().undo.length)

/** The loudest peak `mixer.levels` reads for the track since its last call, in dBFS (`null`: silence). */
async function trackPeak(page: Page, track: string): Promise<number | null> {
  const result = await page.evaluate((c) => (window as unknown as { __faulkAgent(c: object): Promise<unknown> }).__faulkAgent(c), { type: 'mixer.levels' })
  const answer = result as { status: string; output?: { tracks?: { track: string; peak_db: number | null }[] } }
  expect(answer.status).toBe('applied')
  return answer.output?.tracks?.find((t) => t.track === track)?.peak_db ?? null
}

/** Waits until `mixer.levels` hears the track: a peak above -30 dBFS. */
async function heard(page: Page, track: string) {
  // The first call empties what the meters held before.
  await trackPeak(page, track)
  await expect.poll(async () => (await trackPeak(page, track)) ?? -200, { timeout: 15_000, intervals: [300] }).toBeGreaterThan(-30)
}

const audioPlacements = async (page: Page, track: string) =>
  Object.entries((await view(page)).project.tracks[track]?.placements ?? {})
    .map(([id, p]) => ({ id, clip: p.clip, start: p.start, frames: p.span.frames! }))
    .sort((a, b) => a.start - b.start)

const stage = (page: Page) => page.getByTestId('arr-stage')

/** Where a tick and a track's row are on the page. */
async function spot(page: Page) {
  const box = (await stage(page).boundingBox())!
  const ppt = Number(await stage(page).getByTestId('timegrid').getAttribute('data-px-per-tick'))
  const scrollTick = Number((await stage(page).getByTestId('timegrid').getAttribute('data-scroll-tick')) ?? 0)
  const scrollY = Number(await stage(page).getAttribute('data-scroll-y'))
  return {
    x: (tick: number) => box.x + (tick - scrollTick) * ppt,
    y: (row: number) => box.y + RULER + row * LANE - scrollY + LANE / 2,
  }
}

async function drag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }) {
  await page.mouse.move(from.x, from.y)
  await page.mouse.down()
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 4 })
  await page.mouse.move(to.x, to.y, { steps: 4 })
  await page.mouse.up()
}

for (const theme of ['dark', 'light'] as const)
  test(`an audio file is launched from the grid, heard, placed on the timeline, edited by hand, dropped on its lane, and found again after a reload (${theme} theme)`, async ({
    page,
  }) => {
    // Both themes (#687): the waveforms take the track's ink in each.
    await page.addInitScript((t) => localStorage.setItem('faulk.theme', t), theme)
    const seen = watch(page)
    const refused: string[] = []
    page.on('console', (message) => {
      if (message.type() === 'warning' && /refused/.test(message.text())) refused.push(message.text())
    })
    await page.setViewportSize({ width: 1600, height: 900 })
    await page.goto('/app')
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme)
    await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
    await engineUp(page, 'Web')
    test.skip((await hooks(page)) !== 'function', 'the command hooks exist in the dev server only')

    // An audio track (the second row), a scene, and the reference file imported in its Clips panel.
    await page.getByTestId('add-track').click()
    await page.getByTestId('add-track-menu').getByRole('menuitem', { name: 'Audio track' }).click()
    const track = Object.entries((await view(page)).project.tracks).find(([, t]) => t.kind === 'audio')![0]
    await run(page, { type: 'view.select_track', track })
    await run(page, { type: 'scene.create', name: 'A' })
    await page.getByTestId('panel-clips').click()
    const chooser = page.waitForEvent('filechooser')
    await page.getByTestId('audio-import-button').click()
    await (await chooser).setFiles(`${FIXTURES}sine-s16.wav`)
    await expect(page.getByTestId('pool-clip')).toHaveCount(1)
    const clip = Object.keys((await view(page)).project.tracks[track]!.clips)[0]!

    // In the grid: its cell shows the waveform; a click launches it (quantized, AU-4) and the track is heard.
    await run(page, { type: 'clip.audio.set_loop', clip, looping: { start: 0, end: 22_050 } })
    await page.getByTestId('pool-grid').click()
    await expect.poll(async () => (await view(page)).project.tracks[track]!.clips[clip]!.slot).not.toBeNull()
    await page.getByTestId('nav-grid').click()
    const cell = page.locator(`[data-cell][data-clip="${clip}"]`)
    await expect(cell.getByTestId('cell-waveform').getByTestId('waveform')).toHaveAttribute('data-state', 'ready', { timeout: 15_000 })
    await cell.click()
    await expect(cell).toHaveAttribute('data-state', 'playing', { timeout: 15_000 })
    await heard(page, track)
    await run(page, { type: 'transport.stop' })
    await run(page, { type: 'clip.audio.set_loop', clip, looping: null })

    // On the timeline: "Place at the playhead" in the Clips panel puts the grid clip at bar 1, drawn with its waveform.
    await page.getByTestId('nav-arrangement').click()
    await page.getByTestId('panel-clips').click()
    await page.getByTestId('pool-place').first().click()
    await expect.poll(async () => (await audioPlacements(page, track)).length).toBe(1)
    const [placed] = await audioPlacements(page, track)
    expect(placed).toMatchObject({ start: 0, frames: { length: 22_050, offset: 0 } })
    await expect.poll(async () => Number(await stage(page).getAttribute('data-waveforms')), { timeout: 15_000 }).toBe(1)
    // The track plays the arrangement again (the grid held it), and the placement is heard.
    await run(page, { type: 'launch.back_track_to_arrangement', track })
    await run(page, { type: 'transport.play' })
    await heard(page, track)
    await run(page, { type: 'transport.stop' })
    await run(page, { type: 'transport.seek', tick: 0 })

    // By hand, each gesture one history entry: move it one bar later, shorten it by a quarter note, split it in two.
    await page.getByTestId('arr-snap').selectOption('half-beat')
    let g = await spot(page)
    let entries = await historyCount(page)
    await drag(page, { x: g.x(FILE_TICKS / 2), y: g.y(1) }, { x: g.x(FILE_TICKS / 2 + BAR), y: g.y(1) })
    await expect.poll(async () => (await audioPlacements(page, track))[0]?.start).toBe(BAR)
    expect(await historyCount(page)).toBe(entries + 1)
    g = await spot(page)
    entries = await historyCount(page)
    await drag(page, { x: g.x(BAR + FILE_TICKS) - 2, y: g.y(1) }, { x: g.x(BAR + FILE_TICKS - 480), y: g.y(1) })
    await expect.poll(async () => (await audioPlacements(page, track))[0]?.frames.length).toBe(11_025)
    expect(await historyCount(page)).toBe(entries + 1)
    entries = await historyCount(page)
    await run(page, { type: 'transport.seek', tick: BAR + 240 })
    g = await spot(page)
    await page.mouse.click(g.x(BAR + 100), g.y(1))
    await page.getByTestId('arr-split').click()
    await expect
      .poll(async () => (await audioPlacements(page, track)).map((p) => [p.start, p.frames.length, p.frames.offset]))
      .toEqual([
        [BAR, 5_512, 0],
        [BAR + 240, 5_513, 5_512],
      ])
    expect(await historyCount(page)).toBe(entries + 1)

    // A file dropped on the lane two bars on is imported and placed there.
    g = await spot(page)
    const bytes = [...readFileSync(`${FIXTURES}sine-s16.wav`)]
    await page.evaluate(
      ({ bytes, x, y }) => {
        const transfer = new DataTransfer()
        transfer.items.add(new File([Uint8Array.from(bytes)], 'dropped.wav', { type: 'audio/wav' }))
        const target = document.querySelector('[data-testid="arr-stage"]')!
        for (const type of ['dragenter', 'dragover', 'drop'])
          target.dispatchEvent(new DragEvent(type, { dataTransfer: transfer, clientX: x, clientY: y, bubbles: true, cancelable: true }))
      },
      { bytes, x: g.x(3 * BAR + 50), y: g.y(1) },
    )
    await expect.poll(async () => (await audioPlacements(page, track)).length, { timeout: 20_000 }).toBe(3)
    expect((await audioPlacements(page, track)).at(-1)).toMatchObject({ start: 3 * BAR, frames: { length: 22_050, offset: 0 } })
    await expect.poll(async () => Number(await stage(page).getAttribute('data-waveforms')), { timeout: 15_000 }).toBe(3)

    // The clip's gain, typed in the Clips panel: one command.
    const gain = page.getByTestId('pool-gain').first()
    await gain.fill('-6')
    await gain.press('Enter')
    await expect.poll(async () => Object.values((await view(page)).project.tracks[track]!.clips).some((c) => c.content.audio?.gain_db === -6)).toBe(true)

    // After a reload the draft comes back with its three placements, drawn with their waveforms.
    const before = await audioPlacements(page, track)
    await page.waitForTimeout(2_000)
    await page.reload()
    await expect.poll(async () => (await audioPlacements(page, track).catch(() => [])).length, { timeout: 30_000 }).toBe(3)
    expect(await audioPlacements(page, track)).toEqual(before)
    await page.getByTestId('nav-arrangement').click()
    await expect.poll(async () => Number(await stage(page).getAttribute('data-waveforms')), { timeout: 20_000 }).toBe(3)

    if (process.env.FAULK_E2E_SHOTS) {
      g = await spot(page)
      await page.mouse.move(g.x(2 * BAR), g.y(1))
      await page.keyboard.down('Control')
      for (let i = 0; i < 6; i++) await page.mouse.wheel(0, -200)
      await page.keyboard.up('Control')
      await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/audio-clips-timeline-${theme}.png` })
      await page.getByTestId('nav-grid').click()
      await page.getByTestId('panel-clips').click()
      await page.screenshot({ path: `${process.env.FAULK_E2E_SHOTS}/audio-clips-grid-${theme}.png` })
    }
    expect(refused).toEqual([])
    expect(seen.errors).toEqual([])
  })
