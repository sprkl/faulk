import { expect, test, type Page } from '@playwright/test'
import { engineUp, skipWithoutDevHooks, watch } from './support'

// #158: Sluice, a noise gate. Inserted from a track's device panel: one control per parameter of its descriptor, its
// open or closed state and its own reduction shown on its card (not the strip's), the gate following its own track,
// then another track chosen as its detection source (the hat, muted: it still triggers), undo, remove. English and
// French. Real worklet and engine; the dev hooks build the tracks and play the notes, the panel is clicked.

const TEXTS = {
  en: {
    add: 'Add Sluice to the track',
    remove: 'Remove Sluice',
    open: 'Open',
    closed: 'Closed',
    source: 'Detection source',
    labels: ['Open threshold', 'Close threshold', 'Depth', 'Attack', 'Hold', 'Release', 'Invert', 'Listen above', 'Listen below', 'Lookahead'],
  },
  fr: {
    add: 'Ajouter Sluice à la piste',
    remove: 'Retirer Sluice',
    open: 'Ouverte',
    closed: 'Fermée',
    source: 'Source de la détection',
    labels: [
      'Seuil d’ouverture',
      'Seuil de fermeture',
      'Profondeur',
      'Attaque',
      'Maintien',
      'Relâchement',
      'Inversé',
      'Écoute au-dessus de',
      'Écoute en dessous de',
      'Anticipation',
    ],
  },
} as const

type Hooks = {
  __faulkRun: (c: unknown) => unknown
  __faulkView: () => { project: { tracks: Record<string, { name: string; devices: Record<string, { source: unknown; sidechain?: string | null }> }> } }
}

const run = (page: Page, command: unknown) => page.evaluate((c) => (window as unknown as Hooks).__faulkRun(c), command)

async function trackIds(page: Page) {
  return page.evaluate(() => {
    const byName: Record<string, string> = {}
    for (const [id, t] of Object.entries((window as unknown as Hooks).__faulkView().project.tracks)) byName[t.name] = id
    return byName
  })
}

for (const lang of ['en', 'fr'] as const) {
  test(`Sluice shows its own state and reduction and listens to another track, ${lang}`, async ({ page }) => {
    const { errors } = watch(page)
    await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
    await page.goto('/app')
    await page.getByTestId('engine-state').getByRole('button').first().click()
    await engineUp(page, 'Web')
    await skipWithoutDevHooks(page)
    const x = TEXTS[lang]
    const first = Object.keys(await trackIds(page))
    const padId = (await trackIds(page))[first[0]!]!
    await run(page, { type: 'track.rename', track: padId, name: 'Pad' })
    await run(page, { type: 'track.create', kind: 'midi', name: 'Hat' })
    await expect(page.getByTestId('track-list')).toHaveAttribute('data-track-count', '2')

    await page.getByTestId('nav-mixer').click()
    await page.getByTestId('mixer-devices').first().click()
    const panel = page.getByTestId('bottom-panel')
    await panel.getByRole('button', { name: x.add, exact: true }).click()
    // The name is the descriptor's, English in both languages.
    const card = panel.getByTestId('device-card').filter({ hasText: 'Sluice' })
    await expect(card).toHaveAttribute('data-kind', 'faulk.sluice')
    // One control per parameter of the descriptor, with its label in the language.
    await expect(card.locator('[data-testid^=param-]')).toHaveCount(10)
    // A choice is a radio group named by its label, the other controls a text.
    for (const label of x.labels)
      await expect(
        card
          .getByText(label, { exact: true })
          .or(card.getByRole('radiogroup', { name: label, exact: true }))
          .first(),
      ).toBeVisible()
    await expect(card.getByTestId('param-attack').getByRole('slider')).toHaveAttribute('aria-valuetext', /ms/)
    await expect(card.getByTestId('param-depth').getByRole('slider')).toHaveAttribute('aria-valuetext', /dB/)
    await expect(card.getByTestId('param-lookahead').getByRole('slider')).toHaveAttribute('aria-valuetext', /ms/)
    await expect(card.getByTestId('device-sidechain')).toBeVisible()
    await expect(card.getByText(x.source, { exact: true })).toBeVisible()

    // Closed by itself with nothing to hear.
    const state = card.getByTestId('gate-state')
    await expect(state).toHaveText(x.closed, { timeout: 8000 })
    await expect(state).toHaveAttribute('data-state', 'closed')

    // A low opening threshold: a note on the pad opens the gate (its own detection), the reduction falls to 0.
    for (const key of ['param-threshold', 'param-close']) {
      await card.getByTestId(key).getByRole('slider').focus()
      await page.keyboard.press('Home')
    }
    await run(page, { type: 'live.note_on', track: padId, note: 60, velocity: 127 })
    await expect(state).toHaveText(x.open, { timeout: 8000 })
    await expect(state).toHaveAttribute('data-state', 'open')
    await expect(card.getByTestId('gain-reduction-value')).toHaveText('0.0 dB')
    await page.screenshot({ path: `test-results/sluice-open-${lang}.png` })
    await run(page, { type: 'live.note_off', track: padId, note: 60 })

    // Listening to the hat (muted: it still triggers): the pad's note is gated until the hat plays.
    const hat = (await trackIds(page)).Hat!
    await card.getByTestId('device-sidechain').selectOption({ label: 'Hat' })
    await run(page, { type: 'mixer.set_mute', track: hat, mute: true })
    await run(page, { type: 'live.note_on', track: padId, note: 60, velocity: 127 })
    await expect(state).toHaveText(x.closed, { timeout: 8000 })
    await expect
      .poll(async () => Number(((await card.getByTestId('gain-reduction-value').textContent()) ?? '').replace('−', '-').replace(' dB', '')), { timeout: 8000 })
      .toBeLessThan(-20)
    await run(page, { type: 'live.note_on', track: hat, note: 48, velocity: 127 })
    await expect(state).toHaveText(x.open, { timeout: 8000 })
    await expect(card.getByTestId('gain-reduction-value')).toHaveText('0.0 dB', { timeout: 8000 })
    await run(page, { type: 'live.note_off', track: hat, note: 48 })
    await run(page, { type: 'live.note_off', track: padId, note: 60 })
    await page.screenshot({ path: `test-results/sluice-source-${lang}.png` })

    // Bypassed: no state shown.
    await card.getByRole('button', { name: lang === 'en' ? 'Bypass Sluice' : 'Contourner Sluice', exact: true }).click()
    await expect(state).toHaveAttribute('data-state', 'none')
    await card.getByRole('button', { name: lang === 'en' ? 'Bypass Sluice' : 'Contourner Sluice', exact: true }).click()

    // Removed, then undone: the card comes back with its source.
    await card.getByRole('button', { name: x.remove, exact: true }).click()
    await expect(card).toHaveCount(0)
    await run(page, { type: 'history.undo' })
    await expect(card).toHaveCount(1)
    expect(errors).toEqual([])
  })
}
