import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #108: the command palette (Ctrl+K) lists the catalogue's commands, finds them by typing and runs the one chosen through
// the dispatcher: a command without input runs at once, one with input asks for its fields first, and what the project
// refuses is said in the form.

test.use({ storageState: { cookies: [], origins: [] } })
test.describe.configure({ timeout: 90_000 })

const palette = (page: Page) => page.getByTestId('palette-dialog')
const search = (page: Page) => page.getByTestId('palette-search')

async function start(page: Page) {
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

test.describe('the command palette', () => {
  test('Ctrl+K opens a modal dialog on the search field, Escape closes it and gives the focus back', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await expect(palette(page)).toBeHidden()
    await page.keyboard.press('Control+k')
    await expect(palette(page)).toBeVisible()
    await expect(palette(page)).toHaveAttribute('aria-modal', 'true')
    await expect(palette(page)).toHaveAccessibleName('Commands')
    await expect(search(page)).toBeFocused()
    // Every command is listed up to the page's size, and the count says how many there are.
    await expect(page.getByTestId('palette-status')).toContainText(/50 of \d+ commands/)
    await page.keyboard.press('Escape')
    await expect(palette(page)).toBeHidden()
    // In a text field the key is left alone.
    await page.getByTestId('transport-tempo').focus()
    await page.keyboard.press('Control+k')
    await expect(palette(page)).toBeHidden()
    expect(errors).toEqual([])
  })

  test('a command without input runs at once from the keyboard', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('play')
    await expect(page.locator('[role="option"]').first()).toHaveAttribute('data-command', 'transport.play')
    await page.keyboard.press('Enter')
    await expect(palette(page)).toBeHidden()
    await expect(page.getByTestId('transport-play')).toHaveAttribute('aria-pressed', 'true')
    await page.keyboard.press('Control+k')
    await search(page).fill('transport stop')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('transport-play')).toHaveAttribute('aria-pressed', 'false')
    expect(errors).toEqual([])
  })

  test('a command with input asks for it, says what is wrong, then runs and can be undone', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    const before = await page.getByTestId('project-name').innerText()
    await page.keyboard.press('Control+k')
    await search(page).fill('project.rename')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('ArrowUp')
    await page.keyboard.press('Enter')
    const form = page.getByTestId('palette-form')
    await expect(form).toBeVisible()
    await expect(form.locator('[data-field]').first()).toBeFocused()
    // An empty required field is said, and nothing ran.
    await page.getByTestId('palette-run').click()
    await expect(form.getByRole('alert')).toContainText('needs a value')
    await form.locator('[data-field]').first().fill('Palette song')
    await page.keyboard.press('Enter')
    await expect(palette(page)).toBeHidden()
    await expect(page.getByTestId('project-name')).toContainText('Palette song')
    // It went through the dispatcher: the history has it, and Ctrl+Z takes it back.
    await page.keyboard.press('Control+z')
    await expect(page.getByTestId('project-name')).toHaveText(before)
    expect(errors).toEqual([])
  })

  test('a choice field offers the schema options, and Back returns to the list', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('view.set_theme')
    await page.keyboard.press('Enter')
    const select = page.getByTestId('palette-form').locator('select')
    await expect(select.locator('option')).toHaveText(['', 'system', 'dark', 'light'])
    await page.getByTestId('palette-back').click()
    await expect(search(page)).toBeFocused()
    await expect(page.getByTestId('palette-list')).toBeVisible()
  })

  test('before the audio starts a command that cannot run says so and the palette stays open', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    // The page is ready for keys once its engine button is drawn (the audio is not started).
    await expect(page.getByTestId('engine-state').getByRole('button')).toBeVisible()
    await page.keyboard.press('Control+k')
    await expect(palette(page)).toBeVisible()
    await search(page).fill('transport.play')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('palette-refused')).toContainText('Start the audio first')
    await expect(palette(page)).toBeVisible()
    // Typing again clears it.
    await search(page).fill('transport.stop')
    await expect(page.getByTestId('palette-refused')).toHaveCount(0)
    // A command that asks for input says it in its form too.
    await search(page).fill('project.rename')
    await page.keyboard.press('Enter')
    await page.getByTestId('palette-form').locator('[data-field]').first().fill('x')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('palette-refused')).toContainText('Start the audio first')
  })

  test('a destructive command warns in its form, and an empty level is never null', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('project.new')
    await page.keyboard.press('Enter')
    await expect(page.getByTestId('palette-destructive')).toContainText('replaces the project')
    await page.getByTestId('palette-back').click()
    await search(page).fill('mixer.set_volume')
    await page.keyboard.press('Enter')
    await page.getByTestId('palette-run').click()
    await expect(page.getByTestId('palette-form').getByRole('alert').first()).toContainText('needs a value')
    await expect(palette(page)).toBeVisible()
  })

  test('a query with no match says so', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Control+k')
    await search(page).fill('zzzzqqqq')
    await expect(page.getByTestId('palette-status')).toHaveText('No command matches.')
    await page.keyboard.press('Enter')
    await expect(palette(page)).toBeVisible()
  })
})
