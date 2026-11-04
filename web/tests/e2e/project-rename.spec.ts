import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #627: the project is named from the interface: the project menu, a double click or F2 on its name in the top bar. Each way
// goes through `project.rename`, so the name is one entry of the history and Ctrl+Z gives the old one back. No dev hook is
// used by the first three tests, which run the same in the `dev` and the `preview` projects; the fourth (a field left open
// while another project replaces this one) goes through `__faulkRun` and runs in `dev` only.

const name = (page: Page) => page.getByTestId('project-name')

async function open(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  await expect(name(page)).toBeVisible()
}

test('the menu renames the project, and the rename is undone and redone', async ({ page }) => {
  const seen = watch(page)
  await open(page)
  const before = (await name(page).textContent())!
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-rename').click()
  const field = page.getByTestId('project-rename')
  await expect(field).toBeFocused()
  await expect(field).toHaveValue(before)
  await field.fill('Night drive')
  await field.press('Enter')
  await expect(field).toHaveCount(0)
  await expect(name(page)).toHaveText('Night drive')
  // The focus is back on the name, where a shortcut reaches it.
  await expect(page.getByTestId('project-menu')).toBeFocused()

  await page.keyboard.press('Control+z')
  await expect(name(page)).toHaveText(before)
  await page.keyboard.press('Control+Shift+z')
  await expect(name(page)).toHaveText('Night drive')
  expect(seen.errors).toEqual([])
})

test('a double click on the name opens the field; Escape cancels, an empty name changes nothing', async ({ page }) => {
  const seen = watch(page)
  await open(page)
  const before = (await name(page).textContent())!
  await page.getByTestId('project-menu').dblclick()
  const field = page.getByTestId('project-rename')
  await expect(field).toBeFocused()
  // The menu the two clicks toggled is closed.
  await expect(page.getByTestId('menu-rename')).toHaveCount(0)
  await field.fill('Discarded')
  await field.press('Escape')
  await expect(field).toHaveCount(0)
  await expect(name(page)).toHaveText(before)

  await page.getByTestId('project-menu').dblclick()
  await page.getByTestId('project-rename').fill('   ')
  await page.getByTestId('project-rename').press('Enter')
  await expect(name(page)).toHaveText(before)

  // Leaving the field keeps what was typed, as for a track.
  await page.getByTestId('project-menu').dblclick()
  await page.getByTestId('project-rename').fill('Kept')
  await page.getByTestId('nav-arrangement').click()
  await expect(name(page)).toHaveText('Kept')
  await page.keyboard.press('Control+z')
  await expect(name(page)).toHaveText(before)
  expect(seen.errors).toEqual([])
})

test('a field left open while another project replaces this one renames nothing', async ({ page }) => {
  // The replacement goes in through the dev server's hook: the preview build has none, and keeps the checks above.
  test.skip(test.info().project.name !== 'dev', 'needs the dev hook __faulkRun')
  const seen = watch(page)
  await open(page)
  await page.getByTestId('project-menu').dblclick()
  await page.getByTestId('project-rename').fill('Stale')
  await page.evaluate(() => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun({ type: 'project.new', name: 'Fresh' }))
  await expect.poll(() => page.evaluate(() => (window as unknown as { __faulkView(): { project: { name: string } } }).__faulkView().project.name)).toBe('Fresh')
  await page.getByTestId('project-rename').press('Enter')
  await expect(page.getByTestId('project-rename')).toHaveCount(0)
  await expect(name(page)).toHaveText('Fresh')
  expect(seen.errors).toEqual([])
})

test('F2 on the name in the top bar renames the project', async ({ page }) => {
  const seen = watch(page)
  await open(page)
  await page.getByTestId('project-menu').focus()
  await page.keyboard.press('F2')
  const field = page.getByTestId('project-rename')
  await expect(field).toBeFocused()
  await field.fill('Demo')
  await field.press('Enter')
  await expect(name(page)).toHaveText('Demo')
  await expect(page.getByTestId('project-menu')).toHaveAttribute('title', /Demo/)
  await page.keyboard.press('Control+z')
  await expect(name(page)).not.toHaveText('Demo')
  await page.keyboard.press('Control+Shift+z')
  await expect(name(page)).toHaveText('Demo')
  expect(seen.errors).toEqual([])
})
