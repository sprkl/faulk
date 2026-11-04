import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #627: "Save as" asks the project's name first (a field prefilled with the current name), then the folder, and the project is
// made in a folder of that name inside the folder chosen. The folder dialog is simulated: the page's `showDirectoryPicker` hands
// back the directory `parent` of the origin private file system, which is a `FileSystemDirectoryHandle` like the one a real folder
// gives, so the subfolder, the store and the rename all run for real.

test.use({ storageState: { cookies: [], origins: [] } })

const state = (page: Page) => page.getByTestId('save-state')

async function open(page: Page, lang?: 'fr') {
  await page.addInitScript(
    ([parent, language]) => {
      const w = window as unknown as { __picks: number; __faulkNoBrowserStorage: boolean; showDirectoryPicker: () => Promise<FileSystemDirectoryHandle> }
      w.__picks = 0
      w.__faulkNoBrowserStorage = true
      if (language) localStorage.setItem('faulk.lang', language)
      w.showDirectoryPicker = async () => {
        w.__picks++
        return (await navigator.storage.getDirectory()).getDirectoryHandle(parent!, { create: true })
      }
    },
    ['parent', lang],
  )
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

const picks = (page: Page) => page.evaluate(() => (window as unknown as { __picks: number }).__picks)

/** The names inside the OPFS folder `parent`. */
const children = (page: Page, folder = '') =>
  page.evaluate(async (path) => {
    let dir = await (await navigator.storage.getDirectory()).getDirectoryHandle('parent', { create: true })
    for (const part of path.split('/').filter(Boolean)) dir = await dir.getDirectoryHandle(part)
    const out: string[] = []
    for await (const entry of (dir as unknown as { values(): AsyncIterable<FileSystemHandle> }).values()) out.push(entry.name)
    return out.sort()
  }, folder)

async function askName(page: Page) {
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-save-as').click()
  await expect(page.getByTestId('save-as-dialog')).toBeVisible()
}

test('the name is asked first, prefilled; the project is saved in a folder of that name inside the folder chosen', async ({ page }) => {
  const seen = watch(page)
  await open(page)
  const before = (await page.getByTestId('project-name').textContent())!
  await askName(page)
  const field = page.getByTestId('save-as-field')
  await expect(field).toBeFocused()
  await expect(field).toHaveValue(before)
  // No folder dialog yet.
  expect(await picks(page)).toBe(0)

  await field.fill('Night/Drive: 2')
  await page.getByTestId('save-as-submit').click()
  await expect(page.getByTestId('save-as-dialog')).toBeHidden()
  await expect(state(page)).toHaveText(/^Saved at/)
  expect(await picks(page)).toBe(1)
  // The folder name is safe for a file system; the project keeps the name as typed (one `project.rename`).
  expect(await children(page)).toEqual(['Night-Drive- 2'])
  expect(await children(page, 'Night-Drive- 2')).toContain('project.json')
  await expect(page.getByTestId('project-name')).toHaveText('Night/Drive: 2')
  expect(seen.errors).toEqual([])
})

test('Escape and Cancel change nothing, an empty name is refused, and the folder dialog is never opened for them', async ({ page }) => {
  const seen = watch(page)
  await open(page)
  const before = (await page.getByTestId('project-name').textContent())!

  await askName(page)
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('save-as-dialog')).toBeHidden()

  await askName(page)
  await page.getByTestId('save-as-field').fill('   ')
  await page.getByTestId('save-as-submit').click()
  await expect(page.getByTestId('save-as-error')).toBeVisible()
  await expect(page.getByTestId('save-as-dialog')).toBeVisible()
  await page.getByTestId('save-as-cancel').click()
  await expect(page.getByTestId('save-as-dialog')).toBeHidden()

  expect(await picks(page)).toBe(0)
  await expect(page.getByTestId('storage-notice')).toHaveCount(0)
  await expect(state(page)).toHaveText('Not saved')
  await expect(page.getByTestId('project-name')).toHaveText(before)
  expect(seen.errors).toEqual([])
})

test('a folder of that name that holds files is refused with its name, and nothing is renamed or written', async ({ page }) => {
  const seen = watch(page)
  await open(page)
  const before = (await page.getByTestId('project-name').textContent())!
  await page.evaluate(async () => {
    const parent = await (await navigator.storage.getDirectory()).getDirectoryHandle('parent', { create: true })
    const dir = await parent.getDirectoryHandle('Taken', { create: true })
    const writable = await (await dir.getFileHandle('notes.txt', { create: true })).createWritable()
    await writable.write('other files live here')
    await writable.close()
  })
  await askName(page)
  await page.getByTestId('save-as-field').fill('Taken')
  await page.getByTestId('save-as-submit').click()
  await expect(page.getByTestId('storage-notice')).toContainText('“Taken” already exists')
  expect(await children(page, 'Taken')).toEqual(['notes.txt'])
  await expect(state(page)).toHaveText('Not saved')
  await expect(page.getByTestId('project-name')).toHaveText(before)
  expect(seen.errors).toEqual([])
})

test('French: the dialog and the refusal are in French', async ({ page }) => {
  await open(page, 'fr')
  await askName(page)
  await expect(page.getByTestId('save-as-dialog')).toContainText('Nom du projet')
  await page.getByTestId('save-as-field').fill('  ')
  await page.getByTestId('save-as-submit').click()
  await expect(page.getByTestId('save-as-error')).toContainText('Donne un nom au projet.')
})
