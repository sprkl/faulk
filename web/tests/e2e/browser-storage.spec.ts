import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// SV-3 (#96): the browser's own storage (the OPFS) in a real browser. A project with no folder is a
// draft there, offered again after a reload; where the browser has no folder to offer it is the
// project's home; a full storage is said and loses nothing; `.faulkpack` goes out and comes back
// byte for byte. The folder picker is simulated, as in `project-store.spec.ts` (a native dialog
// cannot be driven headless): it hands back a directory of the same file system. Only Chromium runs
// here; see the pull request for what that leaves unverified in Firefox and Safari.

test.describe.configure({ timeout: 90_000 })

const state = (page: Page) => page.getByTestId('save-state')
const tracks = (page: Page) => page.getByTestId('track-header')

async function addTrack(page: Page, item: string) {
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

async function start(page: Page) {
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
}

/** The page's `showDirectoryPicker` hands back the OPFS directory `name`. */
async function simulatePicker(page: Page, name: string) {
  await page.addInitScript((folder) => {
    ;(window as unknown as { showDirectoryPicker: () => Promise<FileSystemDirectoryHandle> }).showDirectoryPicker = async () =>
      (await navigator.storage.getDirectory()).getDirectoryHandle(folder, { create: true })
  }, name)
}

/** A browser with no folder picker at all (Firefox, Safari). */
async function withoutPicker(page: Page) {
  await page.addInitScript(() => Object.defineProperty(window, 'showDirectoryPicker', { value: undefined, configurable: true }))
}

/** The text of every file below the OPFS directory `path`, by path relative to it; `.faulk/` is left out. */
const readTree = (page: Page, path: string[]) =>
  page.evaluate(async (segments) => {
    const out: Record<string, string> = {}
    const walk = async (dir: FileSystemDirectoryHandle, base: string) => {
      for await (const entry of (dir as unknown as { values(): AsyncIterable<FileSystemHandle> }).values()) {
        const at = base ? `${base}/${entry.name}` : entry.name
        if (entry.kind === 'directory') {
          if (at !== '.faulk') await walk(entry as FileSystemDirectoryHandle, at)
        } else {
          // The bytes, as a hash: a text decoding would let two different binary files look alike.
          const bytes = await (await (entry as FileSystemFileHandle).getFile()).arrayBuffer()
          const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
          out[at] = `${bytes.byteLength}:${Array.from(digest, (b) => b.toString(16).padStart(2, '0')).join('')}`
        }
      }
    }
    let dir = await navigator.storage.getDirectory()
    for (const segment of segments) dir = await dir.getDirectoryHandle(segment)
    await walk(dir, '')
    return out
  }, path)

/** The directories of the browser's projects. */
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

/** Whether closing the tab makes the browser ask first. */
async function leaveAsks(page: Page): Promise<boolean> {
  const dialog = page.waitForEvent('dialog', { timeout: 1500 }).then(
    async (d) => {
      const asked = d.type() === 'beforeunload'
      await d.dismiss()
      return asked
    },
    () => false,
  )
  void page.close({ runBeforeUnload: true })
  return dialog
}

const saveAs = async (page: Page) => {
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-save-as').click()
}

test.describe('a draft before a folder is chosen', () => {
  test('a changed project is kept in the browser, the tab can go, and the draft is offered after a reload', async ({ page, context }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await simulatePicker(page, 'song-draft')
    await page.goto('/app')
    await start(page)
    await expect(state(page)).toHaveText('Not saved')
    await addTrack(page, 'Drums')
    await expect(tracks(page)).toHaveCount(2)
    await expect(state(page)).toHaveText(/^Draft saved at \d{1,2}:\d{2}/, { timeout: 20_000 })
    expect(await browserDirs(page)).toHaveLength(1)

    // The draft is a place for the project: the browser does not ask before the tab goes.
    const probe = await context.newPage()
    await simulatePicker(probe, 'song-probe')
    await probe.goto('/app')
    await start(probe)
    await addTrack(probe, 'Drums')
    await expect(state(probe)).toHaveText(/^Draft saved at/, { timeout: 20_000 })
    expect(await leaveAsks(probe)).toBe(false)

    await page.reload()
    await expect(page.getByTestId('reopen-banner')).toContainText('Reopen the draft')
    await expect(tracks(page)).toHaveCount(0)
    await page.getByTestId('reopen').click()
    await expect(tracks(page)).toHaveCount(2)
    await expect(page.getByTestId('reopen-banner')).toHaveCount(0)
    await expect(state(page)).toHaveText(/^Saved/)

    // A change goes on being written to the same draft (the probe's draft is the other one in the list).
    const before = (await browserDirs(page)).length
    await start(page)
    await addTrack(page, 'Oscillator')
    await expect(state(page)).toHaveText(/^Draft saved at/, { timeout: 20_000 })
    expect(await browserDirs(page)).toHaveLength(before)
    expect(errors).toEqual([])
  })

  test('Save as takes the draft into the folder, with the same project, and deletes it', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await simulatePicker(page, 'song-moved')
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText(/^Draft saved at/, { timeout: 20_000 })
    const [dir] = await browserDirs(page)
    const draft = await readTree(page, ['faulk', 'browser', dir!])
    expect(Object.keys(draft)).toContain('project.json')

    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    // The folder holds the very project the draft held, and the draft is gone.
    expect(await readTree(page, ['song-moved'])).toEqual(draft)
    expect(await browserDirs(page)).toEqual([])
    await page.reload()
    await expect(tracks(page)).toHaveCount(2)
    expect(errors).toEqual([])
  })

  test('the banner can delete the draft, after asking; Cancel keeps it', async ({ page }) => {
    await simulatePicker(page, 'song-del')
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText(/^Draft saved at/, { timeout: 20_000 })
    await page.reload()
    await page.getByTestId('delete-draft').click()
    await expect(page.getByTestId('confirm')).toContainText('Delete this project?')
    await page.getByTestId('confirm-cancel').click()
    expect(await browserDirs(page)).toHaveLength(1)
    await page.getByTestId('delete-draft').click()
    await page.getByTestId('confirm-discard').click()
    await expect(page.getByTestId('reopen-banner')).toHaveCount(0)
    await expect.poll(() => browserDirs(page)).toEqual([])
    await page.reload()
    await expect(page.getByTestId('reopen-banner')).toHaveCount(0)
  })

  test('the draft is listed in the menu, and the project can be opened from there', async ({ page }) => {
    await simulatePicker(page, 'song-list')
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText(/^Draft saved at/, { timeout: 20_000 })
    await page.reload()
    await page.getByTestId('reopen').click()
    await expect(tracks(page)).toHaveCount(2)
    await page.getByTestId('project-menu').click()
    await expect(page.getByTestId('menu-browser-project')).toHaveCount(1)
    // The one open cannot be deleted.
    await page.getByTestId('menu-delete-browser-project').click()
    await expect(page.getByTestId('storage-notice')).toContainText('is open')
  })

  test('French: the state and the banner are in French', async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
    await page.setViewportSize({ width: 1440, height: 900 })
    await simulatePicker(page, 'song-fr-draft')
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText(/^Brouillon enregistré à/, { timeout: 20_000 })
    await page.reload()
    await expect(page.getByTestId('reopen-banner')).toContainText('Rouvrir le brouillon')
  })
})

test.describe('a browser with no folders', () => {
  test('the project lives in the browser, comes back by itself, and is exported and imported byte for byte', async ({ page, browser }, testInfo) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await withoutPicker(page)
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(tracks(page)).toHaveCount(2)
    await expect(state(page)).toHaveText(/^Saved in the browser at/, { timeout: 20_000 })
    await expect(page.getByTestId('browser-warning')).toContainText('in the browser')
    await page.getByTestId('project-menu').click()
    await expect(page.getByTestId('menu-unsupported')).toContainText('kept in the browser')
    await expect(page.getByTestId('menu-save-as')).toBeDisabled()
    await page.keyboard.press('Escape')

    // Reload: no question, the project is back.
    await page.reload()
    await expect(tracks(page)).toHaveCount(2)
    await expect(page.getByTestId('reopen-banner')).toHaveCount(0)

    // Export: a download named after the project.
    await page.getByTestId('project-menu').click()
    const [download] = await Promise.all([page.waitForEvent('download'), page.getByTestId('menu-export-pack').click()])
    expect(download.suggestedFilename()).toMatch(/\.faulkpack$/)
    const packPath = testInfo.outputPath('song.faulkpack')
    await download.saveAs(packPath)
    const [dir] = await browserDirs(page)
    const original = await readTree(page, ['faulk', 'browser', dir!])

    // Import in another browser profile, with a folder picker: the pack opens, and saved into a folder
    // gives the very files of the project it was made from.
    const other = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } })
    const second = await other.newPage()
    const secondErrors = watch(second).errors
    await simulatePicker(second, 'song-imported')
    await second.goto('/app')
    await start(second)
    await second.getByTestId('project-menu').click()
    const [chooser] = await Promise.all([second.waitForEvent('filechooser'), second.getByTestId('menu-import-pack').click()])
    await chooser.setFiles(packPath)
    await expect(tracks(second)).toHaveCount(2)
    // The imported project is in the browser's storage, with the identity of the pack.
    const [importedDir] = await browserDirs(second)
    expect(await readTree(second, ['faulk', 'browser', importedDir!])).toEqual(original)
    await saveAs(second)
    await expect(state(second)).toHaveText(/^Saved at/)
    expect(await readTree(second, ['song-imported'])).toEqual(original)
    expect(await browserDirs(second)).toEqual([])
    await other.close()
    expect(secondErrors).toEqual([])
    expect(errors).toEqual([])
  })

  test('a pack that is no pack changes nothing, and says so', async ({ page }, testInfo) => {
    await withoutPicker(page)
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(tracks(page)).toHaveCount(2)
    const bad = testInfo.outputPath('bad.faulkpack')
    const { writeFileSync } = await import('node:fs')
    writeFileSync(bad, 'this is not a zip file')
    await page.getByTestId('project-menu').click()
    const [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.getByTestId('menu-import-pack').click()])
    await chooser.setFiles(bad)
    await expect(page.getByTestId('storage-notice')).toContainText('not a valid .faulkpack')
    await expect(tracks(page)).toHaveCount(2)
  })

  test('with no OPFS either, the project stays in the tab, and the browser warns before it goes', async ({ page }) => {
    const { errors } = watch(page)
    await withoutPicker(page)
    await page.addInitScript(() => Object.defineProperty(navigator.storage, 'getDirectory', { value: undefined, configurable: true }))
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await page.getByTestId('project-menu').click()
    await expect(page.getByTestId('menu-unsupported')).toContainText('cannot save to a folder')
    await expect(page.getByTestId('menu-save-as')).toBeDisabled()
    await page.getByTestId('menu-save').click()
    await expect(page.getByTestId('storage-notice')).toContainText('cannot save to a folder')
    await expect(state(page)).toHaveText('Not saved')
    expect(await leaveAsks(page)).toBe(true)
    expect(errors).toEqual([])
  })
})

test.describe('a full storage', () => {
  test('is said, loses nothing, and the project is written once there is room', async ({ page, context }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await simulatePicker(page, 'song-quota')
    await page.goto('/app')
    await start(page)
    const origin = new URL(page.url()).origin
    const cdp = await context.newCDPSession(page)
    // No room at all: the first write of the draft cannot fit.
    await cdp.send('Storage.overrideQuotaForOrigin', { origin, quotaSize: 1 })
    await addTrack(page, 'Drums')
    await expect(page.getByTestId('full-banner')).toBeVisible({ timeout: 20_000 })
    await expect(state(page)).toHaveText('Could not save')
    // What the failed first write made was taken away again.
    expect(await browserDirs(page)).toEqual([])
    // The only copy is in the tab: the browser asks before it goes.
    expect(await leaveAsks(page)).toBe(true)
  })

  test('Try again writes the draft once there is room', async ({ page, context }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await simulatePicker(page, 'song-quota-retry')
    await page.goto('/app')
    await start(page)
    const origin = new URL(page.url()).origin
    const cdp = await context.newCDPSession(page)
    await cdp.send('Storage.overrideQuotaForOrigin', { origin, quotaSize: 1 })
    await addTrack(page, 'Drums')
    await expect(page.getByTestId('full-banner')).toBeVisible({ timeout: 20_000 })
    await cdp.send('Storage.overrideQuotaForOrigin', { origin })
    await page.getByTestId('full-banner').getByRole('button', { name: 'Try again' }).click()
    await expect(page.getByTestId('full-banner')).toHaveCount(0)
    await expect(state(page)).toHaveText(/^Draft saved at/, { timeout: 20_000 })
  })

  test('a storage that fills up later leaves a draft that opens', async ({ page, context }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await simulatePicker(page, 'song-quota-late')
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText(/^Draft saved at/, { timeout: 20_000 })

    const origin = new URL(page.url()).origin
    const cdp = await context.newCDPSession(page)
    await cdp.send('Storage.overrideQuotaForOrigin', { origin, quotaSize: 1 })
    await addTrack(page, 'Oscillator')
    await expect(page.getByTestId('full-banner')).toBeVisible({ timeout: 20_000 })
    await expect(tracks(page)).toHaveCount(3)

    // The tab goes away with the last edit unwritten (the browser asks first); what is there opens.
    await cdp.send('Storage.overrideQuotaForOrigin', { origin })
    await page.reload()
    await page.getByTestId('reopen').click()
    await expect.poll(() => tracks(page).count()).toBeGreaterThanOrEqual(2)
    expect(await tracks(page).count()).toBeLessThanOrEqual(3)
    await expect(page.getByTestId('storage-notice')).toHaveCount(0)
    expect(errors.filter((e) => !/QuotaExceeded|quota/i.test(e))).toEqual([])
  })
})
