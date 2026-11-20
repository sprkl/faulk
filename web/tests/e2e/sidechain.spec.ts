import { expect, test, type Page } from '@playwright/test'
import { engineUp, skipWithoutDevHooks, watch } from './support'

// #952: a Compressor's detection listening to another track, chosen in its panel by name. The kick (muted: it still
// triggers) turns the silent pad's Compressor down; undo and redo; deleting the kick sets it back to "This track" and
// says so, undo gives it back; a source that already listens to the pad is refused with a message naming it. English
// and French. Real worklet and engine; the dev hooks build the tracks and play the notes, the panel is clicked.

const TEXTS = {
  en: {
    add: 'Add Compressor to the track',
    source: 'Detection source',
    own: 'This track',
    unheard: 'Kick is not heard, but still triggers Compressor.',
    lost: 'Track “Kick” was deleted',
    cycle: '“Kick” already listens to this track',
  },
  fr: {
    add: 'Ajouter Compressor à la piste',
    source: 'Source de la détection',
    own: 'Cette piste',
    unheard: 'Kick n’est pas entendue, mais déclenche toujours Compressor.',
    lost: 'La piste « Kick » a été supprimée',
    cycle: '« Kick » écoute déjà cette piste',
  },
} as const

type Hooks = {
  __faulkRun: (c: unknown) => unknown
  __faulkView: () => {
    project: { tracks: Record<string, { name: string; devices: Record<string, { source: unknown; sidechain?: string | null }> }> }
  }
}

const run = (page: Page, command: unknown) => page.evaluate((c) => (window as unknown as Hooks).__faulkRun(c), command)

/** The ids of the tracks by name, and of the Compressors by their track's name. */
async function ids(page: Page) {
  return page.evaluate(() => {
    const tracks = (window as unknown as Hooks).__faulkView().project.tracks
    const byName: Record<string, string> = {}
    const comp: Record<string, string> = {}
    for (const [id, t] of Object.entries(tracks)) {
      byName[t.name] = id
      for (const [d, dev] of Object.entries(t.devices)) if (JSON.stringify(dev.source).includes('faulk.compressor')) comp[t.name] = d
    }
    return { byName, comp }
  })
}

async function sidechainOf(page: Page, track: string, device: string) {
  return page.evaluate(([t, d]) => (window as unknown as Hooks).__faulkView().project.tracks[t]?.devices[d]?.sidechain ?? null, [track, device])
}

const reductionDb = async (page: Page) =>
  Number(((await page.getByTestId('gain-reduction-value').first().textContent()) ?? '').replace('−', '-').replace(' dB', ''))

for (const lang of ['en', 'fr'] as const) {
  test(`a Compressor listens to another track, ${lang}: chosen by name, muted source, undo, deleted source, a loop refused`, async ({ page }) => {
    const { errors } = watch(page)
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button').first().click()
    await engineUp(page, 'Web')
    await skipWithoutDevHooks(page)
    const x = TEXTS[lang]
    // The first track is the pad; a second one is the kick.
    const first = Object.keys((await ids(page)).byName)
    const padId = (await ids(page)).byName[first[0]!]!
    await run(page, { type: 'track.rename', track: padId, name: 'Pad' })
    await run(page, { type: 'track.create', kind: 'midi', name: 'Kick' })
    await expect(page.getByTestId('track-list')).toHaveAttribute('data-track-count', '2')

    // The pad's devices, the Compressor added from its panel.
    await page.getByTestId('nav-mixer').click()
    await page.getByTestId('mixer-devices').first().click()
    const panel = page.getByTestId('bottom-panel')
    await panel.getByRole('button', { name: x.add, exact: true }).click()
    const card = panel.getByTestId('device-card').filter({ hasText: 'Compressor' })
    const select = card.getByTestId('device-sidechain')
    await expect(select).toBeVisible()
    await expect(card.getByText(x.source, { exact: true })).toBeVisible()
    await expect(select.locator('option')).toHaveText([x.own, 'Kick'])
    await expect(select).toHaveValue('')
    const { byName, comp } = await ids(page)
    const [pad, kick, padComp] = [byName.Pad!, byName.Kick!, comp.Pad!]

    // Chosen by name: the project holds the kick as its source.
    await select.selectOption({ label: 'Kick' })
    await expect.poll(() => sidechainOf(page, pad, padComp)).toBe(kick)
    await expect(select).toHaveValue(kick)

    // A low threshold, the kick muted and played, the pad silent: the reduction follows the kick, and the card says
    // the kick still triggers it.
    await card.getByTestId('param-threshold').getByRole('slider').focus()
    await page.keyboard.press('Home')
    await run(page, { type: 'mixer.set_mute', track: kick, mute: true })
    await expect(card.getByTestId('device-sidechain-unheard')).toHaveText(x.unheard)
    await run(page, { type: 'live.note_on', track: kick, note: 48, velocity: 127 })
    await expect.poll(() => reductionDb(page), { timeout: 8000 }).toBeLessThan(-3)
    await run(page, { type: 'live.note_off', track: kick, note: 48 })
    await expect.poll(() => reductionDb(page), { timeout: 8000 }).toBeGreaterThan(-1)

    // Undo and redo of the choice (the mute and the threshold come back first).
    await page.keyboard.press('Escape')
    for (let i = 0; i < 3; i++) await run(page, { type: 'history.undo' })
    await expect(select).toHaveValue('')
    await expect.poll(() => sidechainOf(page, pad, padComp)).toBe(null)
    await run(page, { type: 'history.redo' })
    await expect(select).toHaveValue(kick)

    // The kick deleted: back to "This track", the page says so; undo brings it back.
    await run(page, { type: 'track.delete', track: kick })
    await expect(page.getByTestId('refusal-toast')).toContainText(x.lost)
    await expect(select).toHaveValue('')
    await expect(select.locator('option')).toHaveText([x.own])
    await run(page, { type: 'history.undo' })
    await expect(select).toHaveValue(kick)
    await expect.poll(() => sidechainOf(page, pad, padComp)).toBe(kick)

    // A loop: the kick's own Compressor listens to the pad, so the pad back on itself, then the kick again, is refused
    // with a message naming the kick.
    await select.selectOption({ label: x.own })
    await expect.poll(() => sidechainOf(page, pad, padComp)).toBe(null)
    await run(page, { type: 'device.insert', track: kick, kind: 'faulk.compressor', index: 0 })
    const kickComp = (await ids(page)).comp.Kick
    await run(page, { type: 'device.set_sidechain', device: kickComp, source: pad })
    await select.selectOption({ label: 'Kick' })
    await expect(page.getByTestId('refusal-toast')).toContainText(x.cycle)
    await expect.poll(() => sidechainOf(page, pad, padComp)).toBe(null)
    await expect(select).toHaveValue('')
    await page.screenshot({ path: `test-results/sidechain-${lang}.png` })
    expect(errors).toEqual([])
  })
}
