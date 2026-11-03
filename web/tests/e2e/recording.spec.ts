import { expect, test, type Page } from '@playwright/test'
import { chooseMidiInput, engineUp, fakeWebMidi, watch } from './support'

// #470: MIDI recording. The armed tracks record what the computer keyboard or a MIDI input plays, the engine stamps the
// notes, and the take becomes clips through one command, one history entry, which one undo removes. Real worklet,
// engine and project core; the project is read back through the dev server's hooks.

interface View {
  project: {
    scenes: Record<string, { name: string; order: string }>
    tracks: Record<
      string,
      {
        kind: string
        clips: Record<string, { slot: string | null; content: { midi?: { notes: { start: number; pitch: number; length: number }[]; length: number } } }>
        placements: Record<string, { clip: string; start: number }>
      }
    >
  }
  session: { armed: string[]; selected_track: string | null }
}

const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun(c), command)
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())
const done = (page: Page) => page.evaluate(() => (window as unknown as { __faulkHistory(): { undo: unknown[] } }).__faulkHistory().undo.length)
const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const record = (page: Page) => page.getByTestId('transport-record')
const feedback = (page: Page) => page.getByTestId('feedback')
const send = (page: Page, bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

/** The first MIDI track and the clips of a track, sorted by id. */
const synth = (v: View) => Object.entries(v.project.tracks).find(([, t]) => t.kind === 'midi')![0]
const clips = (v: View, track: string) => Object.entries(v.project.tracks[track]!.clips)
const notesOf = (v: View, track: string, clip: string) => v.project.tracks[track]!.clips[clip]!.content.midi!.notes

async function start(page: Page, lang: 'en' | 'fr' = 'en') {
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  test.skip((await hooks(page)) !== 'function', 'the project is read through the hooks of the dev server')
  // No count-in: the takes start at once (the count-in is the engine's, tested in crates/engine/tests/recorder.rs).
  await run(page, { type: 'transport.set_metronome', count_in_bars: 0 })
}

/** Arms the first MIDI track from its row in the arrangement (the Rec button). */
async function arm(page: Page, label: RegExp) {
  await page.getByTestId('nav-arrangement').click()
  const rec = page.getByTestId('arr-arm').first()
  await expect(rec).toHaveText('Rec')
  await expect(rec).toHaveAccessibleName(label)
  await rec.click()
  await expect(rec).toHaveAttribute('aria-pressed', 'true')
}

/** Plays a key of the computer keyboard for a moment. */
async function play(page: Page, code: string, ms = 150) {
  await page.keyboard.down(code)
  await page.waitForTimeout(ms)
  await page.keyboard.up(code)
  await page.waitForTimeout(60)
}

test('records a take from the computer keyboard into an empty grid cell; one undo removes it', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  // Nothing armed: the button says why it cannot record.
  await expect(record(page)).toBeDisabled()
  await expect(record(page)).toHaveAttribute('title', 'Arm a MIDI track (Rec) to record it.')
  await arm(page, /^Arm .+ to record$/)
  if (Object.keys((await view(page)).project.scenes).length < 2) {
    await run(page, { type: 'scene.create' })
    await run(page, { type: 'scene.create' })
  }
  // Back to the grid: a click on the empty cell of the second scene makes it the target, shown before the take.
  await page.getByTestId('nav-grid').click()
  const cell = page.getByTestId('cell').and(page.locator('[data-si="1"][data-ti="0"]'))
  await cell.click()
  await expect(cell).toHaveAttribute('data-state', 'empty')
  await expect(page.getByTestId('transport-record-target')).toHaveText(/^Into /)
  const before = await done(page)
  const v0 = await view(page)
  const track = synth(v0)
  const clipsBefore = clips(v0, track).length

  await record(page).click()
  await expect(record(page)).toHaveAttribute('aria-pressed', 'true')
  await expect(feedback(page)).toHaveAttribute('data-transport', 'recording')
  await page.waitForTimeout(300)
  await play(page, 'KeyA')
  await play(page, 'KeyD')
  await play(page, 'KeyG', 250)
  await record(page).click()
  // The take is written once the engine returned its last notes: a new clip in the cell, three notes, one entry.
  await expect.poll(async () => clips(await view(page), track).length).toBe(clipsBefore + 1)
  await expect(record(page)).toHaveAttribute('aria-pressed', 'false')
  const v1 = await view(page)
  const [clipId, clip] = clips(v1, track).find(([id]) => !v0.project.tracks[track]!.clips[id])!
  expect(clip.slot).toBe(Object.entries(v1.project.scenes).sort(([, a], [, b]) => (a.order < b.order ? -1 : 1))[1]![0])
  const notes = notesOf(v1, track, clipId)
  expect(notes.map((n) => n.pitch)).toEqual([60, 64, 67])
  expect(notes.every((n) => n.length > 0 && n.start >= 0)).toBe(true)
  // Ascending, a key after the other.
  expect(notes[1]!.start).toBeGreaterThan(notes[0]!.start)
  expect(await done(page)).toBe(before + 1)

  // One undo removes the whole take.
  await page.keyboard.press('Control+z')
  await expect.poll(async () => clips(await view(page), track).length).toBe(clipsBefore)
  await page.getByTestId('transport-stop').click()
  expect(errors).toEqual([])
})

test('overdubs a looping grid clip: the take is heard over it, drawn live in the piano roll, and joins its notes', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await arm(page, /^Arm .+ to record$/)
  await page.getByTestId('nav-grid').click()
  const v0 = await view(page)
  const track = synth(v0)
  if (Object.keys(v0.project.scenes).length === 0) await run(page, { type: 'scene.create' })
  // A clip of two bars in the first cell, looping, with one note: created like a click in the grid does.
  await page.getByTestId('cell').and(page.locator('[data-si="0"][data-ti="0"]')).dblclick()
  await expect.poll(async () => clips(await view(page), track).length).toBe(1)
  const [clipId] = clips(await view(page), track)[0]!
  await run(page, { type: 'clip.notes.add', clip: clipId, notes: [{ start: 0, pitch: 48, length: 480, velocity: 100 }] })
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '1')
  const cell = page.getByTestId('cell').and(page.locator('[data-si="0"][data-ti="0"]'))
  // The cell is the target: a click would launch it, so it gets the focus from the keyboard.
  await cell.focus()
  await expect(page.getByTestId('transport-record-target')).toHaveText(/^Into /)

  // From a stopped transport the take launches the cell's clip and plays over it.
  await record(page).click()
  await expect(cell).toHaveAttribute('data-state', 'playing')
  await page.waitForTimeout(200)
  await page.keyboard.down('KeyH')
  await page.waitForTimeout(300)
  await page.keyboard.up('KeyH')
  // The finished note is drawn in the piano roll during the take, before it is written.
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-taking', '1')
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '1')
  await record(page).click()
  await expect.poll(async () => notesOf(await view(page), track, clipId).length).toBe(2)
  const notes = notesOf(await view(page), track, clipId)
  expect(notes.map((n) => n.pitch).sort()).toEqual([48, 69])
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-note-count', '2')
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-taking', '0')
  await page.getByTestId('transport-stop').click()
  expect(errors).toEqual([])
})

test('records a simulated MIDI controller into the arrangement, in French; an undo during a take drops it', async ({ page }) => {
  const { errors } = watch(page)
  await fakeWebMidi(page)
  await start(page, 'fr')
  await chooseMidiInput(page, 'E2E Keys')
  await arm(page, /^Armer .+ pour l’enregistrer$/)
  // The arrangement was shown last: the take goes where it is played, from the play position.
  await expect(page.getByTestId('transport-record-target')).toHaveText('À la mesure 1')
  const v0 = await view(page)
  const track = synth(v0)
  const placed = Object.keys(v0.project.tracks[track]!.placements).length
  const before = await done(page)

  // A take dropped by an undo while it runs: nothing is written, nothing enters the history.
  await record(page).click()
  await expect(feedback(page)).toHaveAttribute('data-transport', 'recording')
  await send(page, [0x90, 62, 100])
  await page.waitForTimeout(150)
  await send(page, [0x80, 62, 0])
  await page.keyboard.press('Control+z')
  await expect(record(page)).toHaveAttribute('aria-pressed', 'false')
  await page.getByTestId('transport-stop').click()
  await page.waitForTimeout(300)
  expect(Object.keys((await view(page)).project.tracks[track]!.placements).length).toBe(placed)
  expect(await done(page)).toBe(before)

  // A take from the controller: a new clip placed where it was played.
  await record(page).click()
  await expect(feedback(page)).toHaveAttribute('data-transport', 'recording')
  await page.waitForTimeout(200)
  // The target stays the one of the start while the play position moves on.
  await page.waitForTimeout(2100)
  await expect(page.getByTestId('transport-record-target')).toHaveText('À la mesure 1')
  for (const pitch of [60, 64, 67, 72]) {
    await send(page, [0x90, pitch, 90])
    await page.waitForTimeout(120)
    await send(page, [0x80, pitch, 0])
  }
  // The transport's Stop ends the take too.
  await page.getByTestId('transport-stop').click()
  await expect.poll(async () => Object.keys((await view(page)).project.tracks[track]!.placements).length).toBe(placed + 1)
  const v1 = await view(page)
  const [, placement] = Object.entries(v1.project.tracks[track]!.placements).find(([id]) => !v0.project.tracks[track]!.placements[id])!
  // Played in the second bar (after more than 2 s at 120 BPM): the clip starts at that bar.
  expect(placement.start).toBe(3840)
  expect(notesOf(v1, track, placement.clip).map((n) => n.pitch)).toEqual([60, 64, 67, 72])
  expect(await done(page)).toBe(before + 1)
  await page.keyboard.press('Control+z')
  await expect.poll(async () => Object.keys((await view(page)).project.tracks[track]!.placements).length).toBe(placed)
  expect(errors).toEqual([])
})

test('the default count-in of one bar is shown and its notes are heard, not recorded', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await run(page, { type: 'transport.set_metronome', count_in_bars: 1 })
  await arm(page, /^Arm .+ to record$/)
  const v0 = await view(page)
  const track = synth(v0)
  const placed = Object.keys(v0.project.tracks[track]!.placements).length
  await record(page).click()
  await expect(page.getByTestId('transport-record-target')).toHaveText('Count-in')
  // Played during the count-in: not part of the take.
  await play(page, 'KeyA')
  // The count-in is one bar (2 s at 120 BPM); then the target comes back and the take records.
  await expect(page.getByTestId('transport-record-target')).toHaveText('At bar 1', { timeout: 5000 })
  await play(page, 'KeyD')
  await record(page).click()
  await expect.poll(async () => Object.keys((await view(page)).project.tracks[track]!.placements).length).toBe(placed + 1)
  const v1 = await view(page)
  const [, placement] = Object.entries(v1.project.tracks[track]!.placements).find(([id]) => !v0.project.tracks[track]!.placements[id])!
  expect(notesOf(v1, track, placement.clip).map((n) => n.pitch)).toEqual([64])
  await page.getByTestId('transport-stop').click()
  expect(errors).toEqual([])
})
