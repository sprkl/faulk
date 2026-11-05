import { expect, test, type Page } from '@playwright/test'
import { engineUp, skipWithoutDevHooks, watch } from './support'

// #695: the project's key and scale. The transport's button opens the root and the scale; each choice is
// `project.set_scale` (real project core), kept in the draft across a reload, undone in one step; the piano roll shows
// the scale's rows (#511, its `data-scale` says which, the canvas draws them).

test.describe.configure({ timeout: 90_000 })

const button = (page: Page) => page.getByTestId('transport-scale')

async function start(page: Page, lang: 'en' | 'fr' = 'en') {
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

test('choose F minor with the mouse, see it in the piano roll, find it after a reload, undo it', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await expect(button(page)).toHaveText('Scale')
  await expect(button(page)).toHaveAttribute('aria-label', 'Key and scale: No scale')

  await button(page).click()
  const panel = page.getByTestId('scale-panel')
  await expect(panel).toBeVisible()
  await expect(page.getByTestId('scale-mode')).toHaveValue('chromatic')
  await page.getByTestId('scale-root').selectOption({ label: 'F' })
  // A root alone picks the major scale on it: one command.
  await expect(button(page)).toHaveText('F')
  await page.getByTestId('scale-mode').selectOption('natural_minor')
  await expect(button(page)).toHaveText('Fm')
  await expect(button(page)).toHaveAttribute('aria-label', 'Key and scale: F natural minor')
  // A click elsewhere closes the panel.
  await page.getByTestId('transport-position').click()
  await expect(panel).toHaveCount(0)

  // The piano roll shows F minor's rows (F G A♭ B♭ C D♭ E♭).
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  const roll = page.getByTestId('piano-roll')
  await expect(roll).toHaveAttribute('data-scale', '0,1,3,5,7,8,10')

  // Kept in the draft: after a reload the scale is there before the audio starts.
  await expect(page.getByTestId('save-state')).toHaveText(/^Draft saved at/, { timeout: 20_000 })
  await page.reload()
  await expect(button(page)).toHaveText('Fm')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)

  // Undo: the history of the reloaded project starts empty, so the scale is cleared by choosing none, then undone.
  await button(page).click()
  await page.getByTestId('scale-mode').selectOption('chromatic')
  await expect(button(page)).toHaveText('Scale')
  await page.keyboard.press('Escape')
  await page.keyboard.press('Control+z')
  await expect(button(page)).toHaveText('Fm')
  expect(errors).toEqual([])
})

test('the keyboard opens the panel, picks D Dorian, Escape gives the focus back; undo goes back step by step', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await button(page).focus()
  await page.keyboard.press('Enter')
  const root = page.getByTestId('scale-root')
  await expect(root).toBeFocused()
  await root.selectOption('2')
  await expect(button(page)).toHaveText('D')
  await page.keyboard.press('Tab')
  await expect(page.getByTestId('scale-mode')).toBeFocused()
  await page.getByTestId('scale-mode').selectOption('dorian')
  await expect(button(page)).toHaveText('D dor.')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('scale-panel')).toHaveCount(0)
  await expect(button(page)).toBeFocused()
  await expect(button(page)).toHaveAttribute('data-mode', 'dorian')

  // Two commands, two undos: D major, then no scale.
  await button(page).blur()
  await page.keyboard.press('Control+z')
  await expect(button(page)).toHaveText('D')
  await page.keyboard.press('Control+z')
  await expect(button(page)).toHaveText('Scale')
  await page.keyboard.press('Control+Shift+z')
  await expect(button(page)).toHaveText('D')
  expect(errors).toEqual([])
})

test('the picker speaks French, and a sharp root shows its sign', async ({ page }) => {
  const { errors } = watch(page)
  await start(page, 'fr')
  await expect(button(page)).toHaveText('Gamme')
  await button(page).click()
  await expect(page.getByTestId('scale-panel')).toHaveAttribute('aria-label', 'Tonalité et gamme')
  await expect(page.getByTestId('scale-mode').locator('option').first()).toHaveText('Sans gamme')
  await page.getByTestId('scale-root').selectOption({ label: 'Fa♯' })
  await page.getByTestId('scale-mode').selectOption('harmonic_minor')
  await expect(button(page)).toHaveText('Fa♯ m harm.')
  await expect(button(page)).toHaveAttribute('aria-label', 'Tonalité et gamme : Fa♯ mineur harmonique')
  expect(errors).toEqual([])
})

test('an agent sets the scale through the command, and the piano roll follows', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await skipWithoutDevHooks(page)
  await page.evaluate(() =>
    (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun({ type: 'project.set_scale', root: 9, mode: 'minor_pentatonic' }),
  )
  await expect(button(page)).toHaveText('Am pent.')
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  await expect(page.getByTestId('piano-roll')).toHaveAttribute('data-scale', '0,2,4,7,9')
  expect(errors).toEqual([])
})
