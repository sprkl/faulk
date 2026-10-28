import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// SV-2 (#95): what the automatic save costs on a project of some size, and whether it holds up the
// page while the engine plays. Dev project only (it builds the project through the dispatcher's
// dev hook). A medium project: 16 tracks of 8 clips of 200 notes (25 600 notes). Every number is
// printed; the assertions are ceilings that only catch a gross regression (the autosave runs on the
// page's main thread for the core's part, in a Worker for the writes).

const TRACKS = 16
const CLIPS = 8
const NOTES = 200

type Hooks = {
  __faulkRun: (command: unknown) => void
  __faulkView: () => { project: { tracks: Record<string, { clips: Record<string, unknown> }> } }
  __faulkWrite: () => { coreMs: number; writeMs: number; files: number; bytes: number } | null
}

async function simulatePicker(page: Page, name: string) {
  await page.addInitScript((folder) => {
    // The cost measured is that of a folder; a draft in the browser's storage would write the project first.
    ;(window as unknown as { __faulkNoBrowserStorage: boolean }).__faulkNoBrowserStorage = true
    ;(window as unknown as { showDirectoryPicker: () => Promise<FileSystemDirectoryHandle> }).showDirectoryPicker = async () =>
      (await navigator.storage.getDirectory()).getDirectoryHandle(folder, { create: true })
  }, name)
}

test('the automatic save of a medium project: its cost, and the page and the audio meanwhile', async ({ page }) => {
  test.setTimeout(120_000)
  const { errors } = watch(page)
  await simulatePicker(page, 'song-medium')
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  const hooks = await page.evaluate(() => typeof (window as unknown as Partial<Hooks>).__faulkRun)
  test.skip(hooks !== 'function', 'the dispatcher hook exists in the dev server only')

  // The project, built by commands.
  await page.evaluate(
    ([tracks, clips, notes]) => {
      const w = window as unknown as Hooks
      for (let t = 0; t < tracks!; t++) {
        const before = new Set(Object.keys(w.__faulkView().project.tracks))
        w.__faulkRun({ type: 'track.create', kind: 'midi', instrument: 'faulk.osc_synth', name: `Track ${t}` })
        const track = Object.keys(w.__faulkView().project.tracks).find((id) => !before.has(id))!
        for (let c = 0; c < clips!; c++) {
          const known = new Set(Object.keys(w.__faulkView().project.tracks[track]!.clips))
          w.__faulkRun({ type: 'clip.create', track, at: { at: 'arrangement', start: c * 3840 }, length: 3840 })
          const clip = Object.keys(w.__faulkView().project.tracks[track]!.clips).find((id) => !known.has(id))!
          w.__faulkRun({
            type: 'clip.notes.add',
            clip,
            notes: Array.from({ length: notes! }, (_, n) => ({ start: n * 16, pitch: 36 + (n % 48), length: 12, velocity: 100 })),
          })
        }
      }
    },
    [TRACKS, CLIPS, NOTES],
  )

  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-save-as').click()
  await expect(page.getByTestId('save-state')).toHaveText(/^Saved at/, { timeout: 60_000 })
  const first = await page.evaluate(() => (window as unknown as Hooks).__faulkWrite())
  console.log(`save as, whole project: ${JSON.stringify(first)}`)
  expect(first!.files).toBeGreaterThan(TRACKS * CLIPS)

  // Playing while the autosave runs: what the page and the engine report in the meantime.
  await page.evaluate(() => {
    const w = window as unknown as Hooks & { __probe: { long: number[]; gaps: number[]; last: number } }
    w.__probe = { long: [], gaps: [], last: performance.now() }
    new PerformanceObserver((list) => list.getEntries().forEach((e) => w.__probe.long.push(e.duration))).observe({ entryTypes: ['longtask'] })
    const feedback = document.querySelector('[data-testid=feedback]')!
    new MutationObserver(() => {
      const now = performance.now()
      w.__probe.gaps.push(now - w.__probe.last)
      w.__probe.last = now
    }).observe(feedback, { attributes: true, attributeFilter: ['data-seq'] })
    w.__faulkRun({ type: 'transport.play' })
  })
  await page.waitForTimeout(1500)
  // Playing with nothing to save: the gaps between the engine's reports as they are, to compare with.
  await page.evaluate(() => {
    const p = (window as unknown as { __probe: { long: number[]; gaps: number[] } }).__probe
    p.long.length = 0
    p.gaps.length = 0
  })
  await page.waitForTimeout(6000)
  const idle = await page.evaluate(() => (window as unknown as { __probe: { long: number[]; gaps: number[] } }).__probe)
  const idleWorst = Math.max(...idle.gaps, 0)
  console.log(`playing, nothing to save: engine reports ${idle.gaps.length} in 6 s, longest gap ${idleWorst.toFixed(0)} ms, long tasks ${idle.long.length}`)
  await page.evaluate(() => {
    const p = (window as unknown as { __probe: { long: number[]; gaps: number[] } }).__probe
    p.long.length = 0
    p.gaps.length = 0
  })

  const costs: { coreMs: number; writeMs: number; files: number; bytes: number }[] = []
  for (let i = 0; i < 5; i++) {
    // One note, as an edit while playing: the autosave writes the one file that changed.
    await page.evaluate(() => {
      const w = window as unknown as Hooks
      const view = w.__faulkView()
      const track = Object.keys(view.project.tracks).find((id) => Object.keys(view.project.tracks[id]!.clips).length > 0)!
      const clip = Object.keys(view.project.tracks[track]!.clips)[0]!
      w.__faulkRun({ type: 'clip.notes.add', clip, notes: [{ start: 3000 + Math.floor(Math.random() * 500), pitch: 40, length: 12, velocity: 90 }] })
    })
    await expect(page.getByTestId('save-state')).toHaveText('Unsaved changes')
    await expect(page.getByTestId('save-state')).toHaveText(/^Saved at/, { timeout: 8000 })
    costs.push((await page.evaluate(() => (window as unknown as Hooks).__faulkWrite()))!)
  }
  await page.evaluate(() => (window as unknown as Hooks).__faulkRun({ type: 'transport.stop' }))
  const probe = await page.evaluate(() => (window as unknown as { __probe: { long: number[]; gaps: number[] } }).__probe)
  const worst = (xs: number[]) => (xs.length ? Math.max(...xs) : 0)
  console.log(
    `autosave of one edited note (medium project, ${TRACKS * CLIPS * NOTES} notes), main-thread core ms: ${costs.map((c) => c.coreMs.toFixed(1)).join(', ')}; ` +
      `Worker write ms: ${costs.map((c) => c.writeMs.toFixed(1)).join(', ')}; files ${costs[0]!.files}, bytes ${costs[0]!.bytes}`,
  )
  console.log(
    `main-thread long tasks while playing and saving: ${probe.long.length} (worst ${worst(probe.long).toFixed(0)} ms); engine reports: ${probe.gaps.length}, longest gap ${worst(probe.gaps).toFixed(0)} ms`,
  )
  for (const cost of costs) {
    expect(cost.files).toBe(1)
    expect(cost.coreMs).toBeLessThan(500)
  }
  expect(worst(probe.gaps)).toBeLessThan(500)
  expect(errors).toEqual([])
})
