import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #309: a command the core refuses is shown to the user, in the language of the interface, as a
// temporary toast; repeated refusals share the one toast. The refusal is made through the dev
// server's `__faulkRun` hook: no view sends a command the core refuses by itself.

const startAudio = (page: Page) => page.getByTestId('engine-state').getByRole('button', { name: /Start audio|Démarrer/ }).click()
const missingTrack = () => (window as unknown as { __faulkRun(c: object): void }).__faulkRun({ type: 'track.rename', track: '00000000000000000000000009', name: 'x' })

for (const [locale, text] of [
  ['en-US', 'Not done: what this targets no longer exists.'],
  ['fr-FR', 'Pas fait : la cible n’existe plus.'],
] as const) {
  test(`a refused command is shown (${locale}), grouped when repeated, and goes away`, async ({ browser }, info) => {
    const context = await browser.newContext({ locale, baseURL: info.project.use.baseURL })
    const page = await context.newPage()
    const seen = watch(page)
    await page.goto('/app')
    await startAudio(page)
    await expect(page.getByTestId('engine-mode')).toBeVisible()
    test.skip((await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)) !== 'function', 'the dispatcher hook exists in the dev server only')

    const toast = page.getByTestId('refusal-toast')
    await expect(toast).toHaveCount(0)

    await page.evaluate(missingTrack)
    await expect(toast).toHaveText(text)

    // A burst does not pile up: still one toast, with a count.
    await page.evaluate(() => {
      const run = (window as unknown as { __faulkRun(c: object): void }).__faulkRun
      for (let i = 0; i < 4; i++) run({ type: 'track.rename', track: '00000000000000000000000009', name: 'x' })
    })
    await expect(toast).toHaveCount(1)
    await expect(toast).toHaveText(`${text} (×5)`)

    // It does not block the page: a click goes through it.
    await expect(toast).toHaveCSS('pointer-events', 'none')

    await expect(toast).toHaveCount(0, { timeout: 10_000 })
    // The console keeps the developer's reason (the dispatcher's warning), nothing else is wrong.
    expect(seen.errors).toEqual([])
    await context.close()
  })
}
