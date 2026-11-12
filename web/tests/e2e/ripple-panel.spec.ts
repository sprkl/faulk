import { expect, test, type Page } from '@playwright/test'
import { engineUp, fakeWebMidi, watch } from './support'

// #688: Ripple's panel in a real browser (real worklet, engine and project core). The panel is the
// generated one: in phaser mode it shows the stages, the centre and the spread, in flanger mode the
// delay instead; the stages are a choice of 4, 6, 8 or 12; while the transport plays, the default
// tempo LFO moves the effective centre (phaser) or delay (flanger) on its knob (the engine's readout
// through the report); undo takes the changes back.

const TEXTS = {
  en: { add: 'Add Ripple to the track', phaser: 'Phaser', flanger: 'Flanger', stages: 'Stages', centre: 'Centre', delay: 'Delay' },
  fr: { add: 'Ajouter Ripple à la piste', phaser: 'Phaser', flanger: 'Flanger', stages: 'Étages', centre: 'Fréquence centrale', delay: 'Retard' },
} as const

type Project = { tracks: Record<string, { devices: Record<string, { role: string; params: Record<string, number> }> }> }

const project = (page: Page) => page.evaluate(() => (window as unknown as { __faulkView(): { project: Project } }).__faulkView().project)

/** The params of the track's Ripple (its only effect), as the project holds them. */
async function rippleParams(page: Page): Promise<Record<string, number>> {
  const track = Object.values((await project(page)).tracks)[0]!
  return Object.values(track.devices).find((d) => d.role === 'effect')?.params ?? {}
}

/** The distinct values the live dot of `param`'s knob shows over `ms`. */
async function liveValues(page: Page, param: string, ms: number): Promise<Set<string>> {
  const seen = new Set<string>()
  const dot = page.getByTestId(`param-${param}`).getByTestId('knob-live')
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
  test(`ripple panel, ${lang}: the settings of each mode, the stages, the sweep moving, undo`, async ({ page }) => {
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
    const ripple = page.getByTestId('device-card').filter({ has: page.getByRole('heading', { name: 'Ripple', exact: true }) })
    await expect(ripple).toHaveCount(1)
    const params = ripple.locator('[data-testid^=param-]')

    // Phaser (the default): 19 settings less the delay and the free LFO's rate and restart.
    const mode = ripple.getByTestId('param-mode')
    await expect(mode.getByRole('radio', { name: x.phaser, exact: true })).toHaveAttribute('aria-checked', 'true')
    await expect(params).toHaveCount(16)
    await expect(ripple.getByTestId('param-stages').getByRole('radiogroup', { name: x.stages, exact: true })).toBeVisible()
    await expect(ripple.getByTestId('param-centre').getByRole('slider', { name: x.centre })).toBeVisible()
    await expect(ripple.getByTestId('param-spread')).toBeVisible()
    await expect(ripple.getByTestId('param-delay')).toHaveCount(0)

    // The stages: 4 by default, 12 chosen.
    const stages = ripple.getByTestId('param-stages')
    await expect(stages.getByRole('radio', { name: '4', exact: true })).toHaveAttribute('aria-checked', 'true')
    await stages.getByRole('radio', { name: '12', exact: true }).click()
    await expect(stages.getByRole('radio', { name: '12', exact: true })).toHaveAttribute('aria-checked', 'true')
    await expect.poll(async () => (await rippleParams(page)).stages).toBe(3)

    // Play: the synced LFO (a whole note, two octaves) moves the effective centre on its knob.
    await page.getByTestId('transport-play').click()
    expect((await liveValues(page, 'centre', 2500)).size).toBeGreaterThan(3)

    // Flanger: the delay shows, the phaser's three settings go (19 less 5 with the LFO still synced),
    // and the delay moves.
    await mode.getByRole('radio', { name: x.flanger, exact: true }).click()
    await expect.poll(async () => (await rippleParams(page)).mode).toBe(1)
    await expect(params).toHaveCount(14)
    await expect(ripple.getByTestId('param-delay').getByRole('slider', { name: x.delay })).toBeVisible()
    for (const key of ['stages', 'centre', 'spread']) await expect(ripple.getByTestId(`param-${key}`)).toHaveCount(0)
    expect((await liveValues(page, 'delay', 2500)).size).toBeGreaterThan(3)
    await page.getByTestId('transport-play').click()

    // Undo: the mode, then the stages.
    await page.keyboard.press('Control+z')
    await expect(mode.getByRole('radio', { name: x.phaser, exact: true })).toHaveAttribute('aria-checked', 'true')
    await expect(ripple.getByTestId('param-stages')).toBeVisible()
    await page.keyboard.press('Control+z')
    await expect(ripple.getByTestId('param-stages').getByRole('radio', { name: '4', exact: true })).toHaveAttribute('aria-checked', 'true')
    expect(errors).toEqual([])
  })
}
