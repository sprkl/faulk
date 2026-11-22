import { expect, test, type Page } from '@playwright/test'
import { engineUp, fakeWebMidi, watch } from './support'

// #686: the Filter's movement in a real browser (real worklet, engine and project core). The panel
// is the generated one: its LFO settings follow the LFO's timing, a quarter-note LFO set on the knob
// makes the effective cutoff move on the cutoff knob while the transport plays (the engine's
// readout through the report), a cutoff lane drawn on top keeps it moving, and undo takes it all back.

const TEXTS = {
  en: { add: 'Add Filter to the track', depth: 'LFO depth', free: 'Free', synced: 'Synced', rate: 'LFO rate', notch: 'Notch' },
  fr: { add: 'Ajouter Filter à la piste', depth: 'Profondeur du LFO', free: 'Libre', synced: 'Au tempo', rate: 'Vitesse du LFO', notch: 'Coupe-bande' },
} as const

type Lanes = Record<string, { target: { device?: { device: string; param: string } }; points: Record<string, unknown> }>
type Project = { tracks: Record<string, { automation: Lanes; devices: Record<string, { kind: string }> }> }

const project = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): { project: Project } }).__faulkView().project)
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)

/** The distinct values the live dot of the cutoff knob shows over `ms`. */
async function liveValues(page: Page, ms: number): Promise<Set<string>> {
  const seen = new Set<string>()
  const dot = page.getByTestId('param-cutoff').getByTestId('knob-live')
  const end = Date.now() + ms
  while (Date.now() < end) {
    const v = await dot.getAttribute('data-live').catch(() => null)
    if (v) seen.add(v)
    await page.waitForTimeout(40)
  }
  return seen
}

test.skip(({ browserName }) => Boolean(browserName) && test.info().project.name !== 'dev', 'the dispatcher hooks exist in the dev server only')

for (const lang of ['en', 'fr'] as const) {
  test(`filter movement, ${lang}: a quarter LFO moves the effective cutoff, a lane on top, undo`, async ({ page }) => {
    const x = TEXTS[lang]
    const { errors } = watch(page)
    await fakeWebMidi(page)
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button').click()
    await engineUp(page, 'Web')

    const panel = page.getByTestId('bottom-panel')
    await panel.getByRole('button', { name: x.add, exact: true }).click()
    const filter = page.getByTestId('device-card').filter({ has: page.getByRole('heading', { name: 'Filter', exact: true }) })
    await expect(filter).toHaveCount(1)
    // The generated panel: the notch is a fourth mode, the LFO is synced on a quarter note by default.
    await expect(filter.getByRole('radio', { name: x.notch, exact: true })).toBeVisible()
    await expect(filter.getByRole('radio', { name: '1/4', exact: true })).toHaveAttribute('aria-checked', 'true')
    await expect(filter.getByTestId('param-lfo_rate')).toHaveCount(0)
    // Free: the rate shows, the note value goes; back to synced.
    await filter.getByTestId('param-lfo_sync').getByRole('radio', { name: x.free, exact: true }).click()
    await expect(filter.getByTestId('param-lfo_rate').getByRole('slider', { name: x.rate })).toBeVisible()
    await expect(filter.getByTestId('param-lfo_division')).toHaveCount(0)
    await filter.getByTestId('param-lfo_sync').getByRole('radio', { name: x.synced, exact: true }).click()
    await expect(filter.getByTestId('param-lfo_division')).toBeVisible()

    // Before any depth, the cutoff stands still: the dot sits on the centre.
    const cutoff = filter.getByTestId('param-cutoff')
    await expect(cutoff.getByTestId('knob-live')).toHaveAttribute('data-live', /kHz|Hz/)

    // An octave and a half of LFO, typed on the knob in octaves.
    const depth = filter.getByTestId('param-lfo_depth').getByRole('slider', { name: x.depth })
    await depth.focus()
    await page.keyboard.press('Enter')
    await page.getByTestId('value-input').fill('1.5')
    await page.keyboard.press('Enter')
    await expect(depth).toHaveAttribute('aria-valuetext', '1.5 oct')

    // Play: the effective cutoff moves on the knob, the centre does not.
    const centre = await cutoff.getByRole('slider').getAttribute('aria-valuetext')
    await page.getByTestId('transport-play').click()
    expect((await liveValues(page, 2500)).size).toBeGreaterThan(3)
    await expect(cutoff.getByRole('slider')).toHaveAttribute('aria-valuetext', centre!)

    // A lane on the cutoff, drawn over the LFO (points by command, as an agent would): the dot keeps moving.
    await filter
      .locator('.param-wrap', { has: page.getByTestId('param-cutoff') })
      .getByTestId('device-param-automate')
      .click()
    const lanes = async () => Object.entries(Object.values((await project(page)).tracks)[0]!.automation)
    await expect.poll(async () => (await lanes()).length).toBe(1)
    const [lane] = (await lanes())[0]!
    await run(page, {
      type: 'automation.points.add',
      lane,
      points: [
        { tick: 0, value: 200, shape: 'linear' },
        { tick: 4 * 3840, value: 8000, shape: 'linear' },
      ],
    })
    await expect.poll(async () => Object.keys((await lanes())[0]![1].points).length).toBe(2)
    if ((await panel.getAttribute('data-panel')) !== 'devices')
      await page
        .getByTestId('nav-devices')
        .click()
        .catch(() => {})
    expect((await liveValues(page, 2000)).size).toBeGreaterThan(3)
    await page.getByTestId('transport-play').click()

    // Undo: the points, then the lane, then the depth.
    await page.keyboard.press('Control+z')
    await expect.poll(async () => Object.keys((await lanes())[0]?.[1].points ?? {}).length).toBe(0)
    await page.keyboard.press('Control+z')
    await expect.poll(async () => (await lanes()).length).toBe(0)
    await page.keyboard.press('Control+z')
    await expect(depth).not.toHaveAttribute('aria-valuetext', '1.5 oct')
    expect(errors).toEqual([])
  })
}
