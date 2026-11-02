import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #323: the page's storage actions are commands. Each one is sent through the dispatcher's one entry
// point (`__faulkRun`, dev project only), which has no click behind it, like an agent; and the same
// actions are clicked in the interface, which now sends the same commands. The folder picker and the
// permission are simulated as in `project-store.spec.ts` (a native dialog cannot be driven headless).
// Only Chromium runs here.

test.describe.configure({ timeout: 90_000 })
// The dispatcher's hooks (`__faulkRun`, `__faulkStored`) exist in the dev server only.
test.skip(({ browserName }) => Boolean(browserName) && test.info().project.name !== 'dev', 'the dispatcher hooks exist in the dev server only')

type Faulk = {
  __faulkRun(command: object): void
  __faulkStored(): Promise<{ recent: { id: string; name: string }[]; browser: { dir: string; name: string }[] }>
  __pick: string
  __perm: string
}

const state = (page: Page) => page.getByTestId('save-state')
// The tracks are the arrangement's rows (#471): these tests start on the arrangement, the app's default view.
test.use({ storageState: { cookies: [], origins: [] } })
const tracks = (page: Page) => page.getByTestId('track-header')
const notice = (page: Page) => page.getByTestId('storage-notice')
const run = (page: Page, command: object) => page.evaluate((c) => (window as unknown as Faulk).__faulkRun(c), command)
/**
 * Sends a command as an agent does, with no click behind it. A click keeps the page's "user activation" for a
 * few seconds, and the checks click all the time, so the activation is switched off for this one command.
 */
const runWithoutClick = async (page: Page, command: object) => {
  // The command's outcome is a notice or a question. A previous one still on screen would end the wait at once.
  await expect(page.locator('[data-testid=storage-notice], [data-testid=confirm]')).toHaveCount(0)
  await page.evaluate(async (c) => {
    Object.defineProperty(navigator, 'userActivation', { value: { isActive: false }, configurable: true })
    try {
      ;(window as unknown as Faulk).__faulkRun(c)
      // The storage looks at the activation after some awaits, however long a loaded machine takes to get there: keep it
      // off until the outcome is on screen (a clock would be a guess; with a click-free refusal the notice is that outcome).
      const outcome = '[data-testid=storage-notice], [data-testid=confirm]'
      const end = performance.now() + 30_000
      while (!document.querySelector(outcome) && performance.now() < end) await new Promise((resolve) => setTimeout(resolve, 20))
    } finally {
      delete (navigator as unknown as Record<string, unknown>).userActivation
    }
  }, command)
}
const stored = (page: Page) => page.evaluate(() => (window as unknown as Faulk).__faulkStored())

async function addTrack(page: Page, item: string) {
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

async function start(page: Page) {
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
}

/** The picker hands back the OPFS directory `window.__pick`; the permission is `window.__perm` (granted unless a test says otherwise). */
async function simulate(page: Page, first: string, options: { browserStorage: boolean; lang?: 'fr' }) {
  await page.addInitScript(
    ([folder, browserStorage, lang]) => {
      const w = window as unknown as Faulk & {
        __allow: boolean
        showDirectoryPicker: () => Promise<FileSystemDirectoryHandle>
        __faulkNoBrowserStorage?: boolean
      }
      w.__pick = folder as string
      w.__perm = sessionStorage.getItem('perm') ?? 'granted'
      w.__allow = true
      if (!browserStorage) w.__faulkNoBrowserStorage = true
      if (lang) localStorage.setItem('faulk.lang', lang as string)
      w.showDirectoryPicker = async () => (await navigator.storage.getDirectory()).getDirectoryHandle(w.__pick, { create: true })
      Object.defineProperty(FileSystemHandle.prototype, 'queryPermission', { value: async () => w.__perm, configurable: true })
      Object.defineProperty(FileSystemHandle.prototype, 'requestPermission', {
        value: async () => {
          if (w.__allow) w.__perm = 'granted'
          return w.__perm
        },
        configurable: true,
      })
    },
    [first, options.browserStorage, options.lang ?? ''] as const,
  )
}

const browserDirs = (page: Page) =>
  page.evaluate(async () => {
    try {
      const root = await (await (await navigator.storage.getDirectory()).getDirectoryHandle('faulk')).getDirectoryHandle('browser')
      const names: string[] = []
      for await (const entry of (root as unknown as { values(): AsyncIterable<FileSystemHandle> }).values()) names.push(entry.name)
      return names
    } catch {
      return []
    }
  })

const readText = (page: Page, folder: string, path: string) =>
  page.evaluate(
    async ([name, file]) => {
      try {
        const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(name!)
        return await (await (await dir.getFileHandle(file!)).getFile()).text()
      } catch {
        return null
      }
    },
    [folder, path],
  )

/** Rewrites the project name inside the folder, as an editor or a git checkout would. */
const editOutside = (page: Page, folder: string, name: string) =>
  page.evaluate(
    async ([dirName, title]) => {
      const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(dirName!)
      const handle = await dir.getFileHandle('project.json')
      const project = JSON.parse(await (await handle.getFile()).text())
      project.name = title
      const writable = await handle.createWritable()
      await writable.write(JSON.stringify(project, null, 2) + '\n')
      await writable.close()
      window.dispatchEvent(new Event('focus'))
    },
    [folder, name],
  )

const saveAs = async (page: Page) => {
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-save-as').click()
}

test.describe('the projects the page keeps, by command', () => {
  test('list_stored, reopen_last, open_browser_project, delete_browser_project: a draft is found, reopened and deleted only after the question', async ({
    page,
  }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await simulate(page, 'cmd-draft', { browserStorage: true })
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText(/^Draft saved at/, { timeout: 20_000 })
    const [dir] = await browserDirs(page)
    await page.reload()
    // The draft comes back by itself (#474): no banner, and it is the open project.
    await expect(tracks(page)).toHaveCount(2)
    await expect(page.getByTestId('reopen-banner')).toHaveCount(0)

    // The list the commands take their names from, newest first.
    const listed = await stored(page)
    expect(listed.browser.map((p) => p.dir)).toEqual([dir])
    expect(listed.recent).toEqual([])

    // A name the page never listed is refused with a notice, and nothing is asked of the user.
    await run(page, { type: 'project.delete_browser_project', dir: 'not-a-project' })
    await expect(notice(page)).toContainText('no longer in the browser')
    await expect(page.getByTestId('confirm')).toHaveCount(0)
    await run(page, { type: 'project.open_browser_project', dir: '../escape' })
    await expect(notice(page)).toContainText('no longer in the browser')
    expect(await browserDirs(page)).toEqual([dir])
    // A name the command itself refuses (empty) never gets to the storage.
    await run(page, { type: 'project.delete_browser_project', dir: '' })
    expect(await browserDirs(page)).toEqual([dir])
    // The open project cannot be deleted by a command.
    await run(page, { type: 'project.delete_browser_project', dir: dir! })
    await expect(notice(page)).toContainText('is open')
    expect(await browserDirs(page)).toEqual([dir])

    // Nothing waits for a click, so there is nothing to reopen: said, not silent.
    await run(page, { type: 'project.reopen_last' })
    await expect(notice(page)).toContainText('no project to reopen')

    // Away from it, the draft is one of the list: deleting asks, whoever sends it.
    await run(page, { type: 'project.new' })
    await expect(tracks(page)).toHaveCount(1)
    await run(page, { type: 'project.delete_browser_project', dir: dir! })
    await expect(page.getByTestId('confirm')).toContainText('Delete this project?')
    await page.getByTestId('confirm-cancel').click()
    expect(await browserDirs(page)).toEqual([dir])
    // The command opens it again.
    await run(page, { type: 'project.open_browser_project', dir: dir! })
    await expect(tracks(page)).toHaveCount(2)
    await run(page, { type: 'project.new' })
    await run(page, { type: 'project.delete_browser_project', dir: dir! })
    await page.getByTestId('confirm-discard').click()
    await expect.poll(() => browserDirs(page)).toEqual([])
    expect((await stored(page)).browser).toEqual([])
    expect(errors).toEqual([])
  })

  test('open_folder and open_recent: the chooser needs a click, and the menu clicks send the commands', async ({ page }) => {
    const { errors } = watch(page)
    await simulate(page, 'cmd-a', { browserStorage: false })
    await page.goto('/app')
    await start(page)
    const set = (key: '__pick', value: string) => page.evaluate(([k, v]) => ((window as unknown as Record<string, string>)[k!] = v!), [key, value])
    const name = () => page.getByTestId('project-name')

    await run(page, { type: 'project.rename', name: 'Alpha' })
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    await page.getByTestId('project-menu').click()
    await page.getByTestId('menu-new').click()
    // A new project is named after the moment it started (#473).
    await expect(name()).toHaveText(/^Sketch /)
    await run(page, { type: 'project.rename', name: 'Beta' })
    await set('__pick', 'cmd-b')
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)

    const listed = await stored(page)
    expect(listed.recent.map((r) => r.name)).toEqual(['cmd-b', 'cmd-a'])

    // With no click behind it, the folder chooser is not opened: a notice says why, and nothing changes.
    await runWithoutClick(page, { type: 'project.open_folder' })
    await expect(notice(page)).toContainText('only after a click')
    await expect(name()).toHaveText('Beta')
    await page.getByTestId('storage-notice').getByRole('button').click()
    // A name the page does not know is refused.
    await run(page, { type: 'project.open_recent', id: 'not-a-folder' })
    await expect(notice(page)).toContainText('no longer in the recent projects')
    await page.getByTestId('storage-notice').getByRole('button').click()

    // A click on the menu sends the command: the recent folder opens, then the chosen one.
    await page.getByTestId('project-menu').click()
    await page.getByTestId('menu-recent').filter({ hasText: 'cmd-a' }).click()
    await expect(name()).toHaveText('Alpha')
    await set('__pick', 'cmd-b')
    await page.getByTestId('project-menu').click()
    await page.getByTestId('menu-open').click()
    await expect(name()).toHaveText('Beta')
    expect(errors).toEqual([])
  })
})

// #346: a new project asks before the core replaces anything, whoever asks; save as needs a click.
for (const lang of ['en', 'fr'] as const) {
  test.describe(`new project and save as, ${lang}`, () => {
    test('the menu and the command ask about unsaved changes; Cancel keeps the project; save as without a click says why', async ({ page }) => {
      const { errors } = watch(page)
      await simulate(page, 'cmd-new', { browserStorage: false, lang: lang === 'fr' ? 'fr' : undefined })
      await page.goto('/app')
      await start(page)
      await addTrack(page, 'Drums')
      await expect(tracks(page)).toHaveCount(2)
      const asked = page.getByTestId('confirm')

      // The menu sends the command: the question, then Cancel keeps everything.
      await page.getByTestId('project-menu').click()
      await page.getByTestId('menu-new').click()
      await expect(asked).toBeVisible()
      await expect(asked).toContainText(lang === 'fr' ? 'nouveau projet' : 'new project')
      await page.getByTestId('confirm-cancel').click()
      await expect(asked).toHaveCount(0)
      await expect(tracks(page)).toHaveCount(2)

      // The command with no click behind it asks the same question; Cancel changes nothing, confirming replaces.
      await runWithoutClick(page, { type: 'project.new' })
      await expect(asked).toBeVisible()
      await page.getByTestId('confirm-cancel').click()
      await expect(tracks(page)).toHaveCount(2)
      await runWithoutClick(page, { type: 'project.new' })
      await page.getByTestId('confirm-discard').click()
      await expect(asked).toHaveCount(0)
      await expect(tracks(page)).toHaveCount(1)

      // Save as with no click: no chooser, a notice, the project stays unsaved.
      await runWithoutClick(page, { type: 'project.save_as' })
      await expect(notice(page)).toContainText(lang === 'fr' ? 'après un clic' : 'only after a click')
      await expect(state(page)).toHaveText(lang === 'fr' ? 'Non enregistré' : 'Not saved')
      expect(errors).toEqual([])
    })
  })
}

test.describe('the conflict and the permission, by command', () => {
  test('keep_mine and load_folder_version change nothing without a conflict, and ask before they lose anything', async ({ page }) => {
    const { errors } = watch(page)
    await simulate(page, 'cmd-conflict', { browserStorage: false })
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    const before = await readText(page, 'cmd-conflict', 'project.json')

    // No conflict: refused with a notice, no question, the folder and the project untouched.
    await addTrack(page, 'Drums')
    await run(page, { type: 'project.keep_mine' })
    await expect(notice(page)).toContainText('Nothing was changed')
    await page.getByTestId('storage-notice').getByRole('button').click()
    await run(page, { type: 'project.load_folder_version' })
    await expect(notice(page)).toContainText('Nothing was changed')
    await expect(page.getByTestId('confirm')).toHaveCount(0)
    await expect(tracks(page)).toHaveCount(2)

    // A conflict: the folder is edited from outside while the project has unsaved changes.
    await expect(state(page)).toHaveText(/^Saved at/, { timeout: 8000 })
    await addTrack(page, 'Oscillator')
    await editOutside(page, 'cmd-conflict', 'Edited elsewhere')
    await expect(page.getByTestId('external-banner')).toBeVisible({ timeout: 8000 })
    const outside = await readText(page, 'cmd-conflict', 'project.json')
    expect(outside).not.toBe(before)

    // Load the folder's version: the question comes first, and Cancel keeps the changes.
    await run(page, { type: 'project.load_folder_version' })
    await expect(page.getByTestId('confirm')).toContainText('Load the folder’s version?')
    await page.getByTestId('confirm-cancel').click()
    await expect(page.getByTestId('external-banner')).toBeVisible()
    await expect(tracks(page)).toHaveCount(3)

    // Keep mine: the question names what is lost; Cancel leaves the folder as it is.
    await run(page, { type: 'project.keep_mine' })
    await expect(page.getByTestId('confirm')).toContainText('Replace the folder’s version with yours?')
    await page.getByTestId('confirm-cancel').click()
    expect(await readText(page, 'cmd-conflict', 'project.json')).toBe(outside)

    // Answered yes, the folder's version comes in and the unsaved track goes.
    await run(page, { type: 'project.load_folder_version' })
    await page.getByTestId('confirm-discard').click()
    await expect(page.getByTestId('external-banner')).toHaveCount(0)
    await expect(page.getByTestId('project-name')).toHaveText('Edited elsewhere')
    await expect(tracks(page)).toHaveCount(2)

    // And again, to keep mine: the folder is replaced by the project.
    await addTrack(page, 'Oscillator')
    await editOutside(page, 'cmd-conflict', 'Edited twice')
    await expect(page.getByTestId('external-banner')).toBeVisible({ timeout: 8000 })
    await run(page, { type: 'project.keep_mine' })
    await page.getByTestId('confirm-discard').click()
    await expect(page.getByTestId('external-banner')).toHaveCount(0)
    await expect(state(page)).toHaveText(/^Saved at/, { timeout: 8000 })
    expect(JSON.parse((await readText(page, 'cmd-conflict', 'project.json'))!).name).toBe('Edited elsewhere')
    expect(errors).toEqual([])
  })

  test('allow_access needs a click: without one the browser is not asked, with the banner button it is', async ({ page }) => {
    const { errors } = watch(page)
    await simulate(page, 'cmd-perm', { browserStorage: false })
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    await page.evaluate(() => ((window as unknown as Faulk).__perm = 'prompt'))
    await addTrack(page, 'Drums')
    await expect(page.getByTestId('permission-banner')).toBeVisible({ timeout: 8000 })

    await runWithoutClick(page, { type: 'project.allow_access' })
    // One banner at a time (#474): the permission banner, whose button is the remedy, hides the notice about the click.
    await expect(page.getByTestId('permission-banner')).toBeVisible()
    await expect(notice(page)).toHaveCount(0)
    expect(await page.evaluate(() => (window as unknown as Faulk).__perm)).toBe('prompt')

    await page.getByTestId('allow').click()
    await expect(page.getByTestId('permission-banner')).toHaveCount(0)
    await expect(state(page)).toHaveText(/^Saved at/, { timeout: 8000 })
    expect(errors).toEqual([])
  })
})

test.describe('in French', () => {
  test('the questions, the notices and the banners of the commands are in French', async ({ page }) => {
    const { errors } = watch(page)
    await simulate(page, 'cmd-fr', { browserStorage: false, lang: 'fr' })
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Enregistré à/)

    await run(page, { type: 'project.keep_mine' })
    await expect(notice(page)).toContainText('Rien n’a été changé')
    await page.getByTestId('storage-notice').getByRole('button').click()
    await runWithoutClick(page, { type: 'project.open_folder' })
    await expect(notice(page)).toContainText('qu’après un clic')
    await page.getByTestId('storage-notice').getByRole('button').click()
    await run(page, { type: 'project.open_recent', id: 'inconnu' })
    await expect(notice(page)).toContainText('plus dans les projets récents')

    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText(/^Enregistré à/, { timeout: 8000 })
    await addTrack(page, 'Oscillator')
    await editOutside(page, 'cmd-fr', 'Modifié ailleurs')
    await expect(page.getByTestId('external-banner')).toBeVisible({ timeout: 8000 })
    await page.getByTestId('keep-mine').click()
    await expect(page.getByTestId('confirm')).toContainText('Remplacer la version du dossier par la tienne ?')
    await page.getByTestId('confirm-cancel').click()
    await page.getByTestId('load-disk').click()
    await expect(page.getByTestId('confirm')).toContainText('Charger la version du dossier ?')
    await expect(page.getByTestId('confirm')).toContainText('Abandonner mes modifications')
    await page.getByTestId('confirm-cancel').click()
    expect(errors).toEqual([])
  })
})
