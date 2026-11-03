import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #473: the history names what each entry acted on ("Mute Synth", "Delete 6 clips (Bass, Drums, Synth)"), the
// transport's switches leave the undo history for its activity log, a solo is undone like a mute, a new project is
// named, and a project comes back with its layout after a reload.

test.describe.configure({ timeout: 120_000 })
test.use({ storageState: { cookies: [], origins: [] } })

const hooks = (page: Page) => page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
const run = (page: Page, command: object, author?: string) =>
  page.evaluate(([c, a]) => (window as unknown as { __faulkRun(c: object, a?: string): void }).__faulkRun(c as object, a as string | undefined), [
    command,
    author,
  ] as const)
type View = {
  project: {
    id: string
    name: string
    transport: { loop_enabled: boolean }
    tracks: Record<string, { name: string; placements: Record<string, unknown> }>
  }
  session: { solo: string[] }
}
const view = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): View }).__faulkView())

async function start(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

async function trackNamed(page: Page, name: string): Promise<string> {
  await run(page, { type: 'track.create', kind: 'midi', name })
  await expect.poll(async () => Object.entries((await view(page)).project.tracks).find(([, t]) => t.name === name)?.[0]).toBeTruthy()
  return Object.entries((await view(page)).project.tracks).find(([, t]) => t.name === name)![0]
}

test('history names its objects; transport switches are listed but not undone; solo is undone', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the command hook exists in the dev server only')

  // A new project has a name, never "Untitled".
  expect((await view(page)).project.name).toMatch(/^Sketch /)
  await expect(page.getByTestId('project-name')).toHaveText(/^Sketch /)

  const synth = Object.keys((await view(page)).project.tracks)[0]!
  const bass = await trackNamed(page, 'Bass')
  const drums = await trackNamed(page, 'Drums')
  for (const track of [synth, bass, drums]) {
    for (const start of [0, 3840]) await run(page, { type: 'clip.create', track, at: { at: 'arrangement', start } })
  }
  await expect.poll(async () => Object.values((await view(page)).project.tracks).reduce((n, t) => n + Object.keys(t.placements).length, 0)).toBe(6)

  await run(page, { type: 'mixer.set_mute', track: synth, mute: true })
  const placements = Object.values((await view(page)).project.tracks).flatMap((t) => Object.keys(t.placements))
  await run(page, { type: 'arrangement.delete', placements })

  await page.getByTestId('panel-history').click()
  const rows = page.locator('[data-testid=history-entry], [data-testid=history-activity]')
  await expect(rows.first()).toHaveText(/^Delete 6 clips \(Synth, Bass, Drums\)/)
  await expect(rows.nth(1)).toHaveText(/^Mute Synth/)
  await expect(page.getByTestId('history-entry').filter({ hasText: 'Create Drums 2 (Drums)' })).toHaveCount(1)

  // The loop switch: a row of the activity log, not undoable; undo goes past it.
  await run(page, { type: 'transport.set_loop', loop_region: { start: 0, end: 7680 } })
  await run(page, { type: 'transport.set_tempo', bpm: 96 })
  await run(page, { type: 'transport.toggle_loop' })
  await expect(rows.first()).toHaveText(/^Turn the loop on\s*Not undoable/)
  await expect(rows.first()).toHaveAttribute('data-testid', 'history-activity')
  await page.getByTestId('history-undo').click()
  await expect(page.getByTestId('history-entry').first()).toHaveAttribute('data-undone', 'true')
  await expect(page.getByTestId('history-entry').first()).toHaveText(/^Set the tempo to 96 BPM/)
  expect((await view(page)).project.transport.loop_enabled).toBe(true)
  await page.getByTestId('history-redo').click()

  // Solo is undone like a mute.
  await run(page, { type: 'mixer.set_solo', track: bass, solo: true })
  await expect(rows.first()).toHaveText(/^Solo Bass/)
  await page.getByTestId('history-undo').click()
  await expect.poll(async () => (await view(page)).session.solo).toEqual([])

  // An agent's entry is named the same way, with the agent as author.
  await run(page, { type: 'track.rename', track: drums, name: 'Perc' }, 'agent:Claude')
  await expect(rows.first()).toHaveText(/^Rename Drums to Perc\s*Claude \(agent\)/)

  expect(errors).toEqual([])
})

test('a project comes back with its view, zoom and open lanes after a reload', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  test.skip((await hooks(page)) !== 'function', 'the command hook exists in the dev server only')
  const stage = page.getByTestId('arr-stage').getByTestId('timegrid')
  await expect.poll(async () => Number(await stage.getAttribute('data-draw-count'))).toBeGreaterThan(0)
  const id = (await view(page)).project.id

  // Zoom in twice, open the synth's lanes.
  const zoom = async () => Number(await stage.getAttribute('data-px-per-tick'))
  const before = await zoom()
  await page.getByTestId('arr-zoom-in').click()
  await page.getByTestId('arr-zoom-in').click()
  const zoomed = await zoom()
  expect(zoomed).toBeGreaterThan(before)
  const synth = Object.keys((await view(page)).project.tracks)[0]!
  await page.locator(`[data-testid=track-header][data-track="${synth}"] [data-testid=arr-automation-toggle]`).click()
  await expect(page.locator(`[data-testid=track-header][data-track="${synth}"] [data-testid=arr-automation-toggle]`)).toHaveAttribute('aria-expanded', 'true')
  // An edit makes the project a draft worth keeping.
  await run(page, { type: 'track.create', kind: 'midi', name: 'Bass' })
  await expect(page.getByTestId('save-state')).toHaveText(/^Draft saved at/, { timeout: 20_000 })

  await page.reload()
  await page.getByTestId('engine-state').getByRole('button').click()
  // The draft comes back on its own, or from the banner that offers it.
  const reopen = page.getByTestId('reopen')
  await expect.poll(async () => (await reopen.isVisible()) || (await view(page))?.project.id === id).toBe(true)
  if (await reopen.isVisible()) await reopen.click()
  await expect.poll(async () => (await view(page)).project.id).toBe(id)
  await expect.poll(zoom).toBeCloseTo(zoomed, 6)
  await expect(page.locator(`[data-testid=track-header][data-track="${synth}"] [data-testid=arr-automation-toggle]`)).toHaveAttribute('aria-expanded', 'true')
  expect(errors).toEqual([])
})
