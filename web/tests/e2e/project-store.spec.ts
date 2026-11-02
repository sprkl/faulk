import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// SV-2 (#95): the project in a real folder, in a real browser. The folder picker is simulated: the
// page's `showDirectoryPicker` hands back a directory of the origin private file system, which is a
// `FileSystemDirectoryHandle` like the one the user's folder gives (the same writable streams, the
// same swap-file replacement), so the project store, its Worker, the autosave, the permission and
// the reload all run for real. A native folder dialog cannot be driven headless; see the pull
// request for what that leaves unverified.

const state = (page: Page) => page.getByTestId('save-state')
// The tracks are the arrangement's rows (#471): these tests start on the arrangement, the app's default view.
test.use({ storageState: { cookies: [], origins: [] } })
const tracks = (page: Page) => page.getByTestId('track-header')

async function addTrack(page: Page, item: string) {
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

async function start(page: Page) {
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
}

/** The picker of the page returns the OPFS directory `name` (made if need be); `mode` can make it fail. */
async function simulatePicker(page: Page, name: string) {
  await page.addInitScript((folder) => {
    const w = window as unknown as { __pick: string; __pickError?: string; showDirectoryPicker: () => Promise<FileSystemDirectoryHandle> }
    w.__pick = folder
    // These checks are about folders: no drafts in the browser's storage (browser-storage.spec.ts covers those).
    ;(window as unknown as { __faulkNoBrowserStorage: boolean }).__faulkNoBrowserStorage = true
    w.showDirectoryPicker = async () => {
      if (w.__pickError) throw new DOMException('simulated', w.__pickError)
      return (await navigator.storage.getDirectory()).getDirectoryHandle(w.__pick, { create: true })
    }
  }, name)
}

/** Simulates the permission of every folder: `granted` unless the test says `prompt` or `denied`. */
async function simulatePermission(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as { __perm: string; __allow: boolean }
    w.__perm = sessionStorage.getItem('perm') ?? 'granted'
    w.__allow = true
    Object.defineProperty(FileSystemHandle.prototype, 'queryPermission', { value: async () => w.__perm, configurable: true })
    Object.defineProperty(FileSystemHandle.prototype, 'requestPermission', {
      value: async () => {
        if (w.__allow) w.__perm = 'granted'
        return w.__perm
      },
      configurable: true,
    })
  })
}

/** The text of a file of the OPFS folder `name`, or null. */
const readText = (page: Page, folder: string, path: string) =>
  page.evaluate(
    async ([name, file]) => {
      try {
        let dir = await (await navigator.storage.getDirectory()).getDirectoryHandle(name!)
        const parts = file!.split('/')
        for (const part of parts.slice(0, -1)) dir = await dir.getDirectoryHandle(part)
        return await (await (await dir.getFileHandle(parts[parts.length - 1]!)).getFile()).text()
      } catch {
        return null
      }
    },
    [folder, path],
  )

/** Every file path of the OPFS folder `name`. */
const listFolder = (page: Page, folder: string) =>
  page.evaluate(async (name) => {
    const out: string[] = []
    const walk = async (dir: FileSystemDirectoryHandle, base: string) => {
      for await (const entry of (dir as unknown as { values(): AsyncIterable<FileSystemHandle> }).values()) {
        const path = base ? `${base}/${entry.name}` : entry.name
        if (entry.kind === 'directory') await walk(entry as FileSystemDirectoryHandle, path)
        else out.push(path)
      }
    }
    await walk(await (await navigator.storage.getDirectory()).getDirectoryHandle(name), '')
    return out.sort()
  }, folder)

/**
 * Whether closing the tab makes the browser ask first (its own `beforeunload` dialog). The question is
 * answered "stay", so the page lives on; `false` when no question comes within a second.
 */
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

test.describe('the project in a folder', () => {
  test('edit, save as, edit, reload: the project is found again, and the state tells the truth', async ({ page, context }) => {
    const { errors } = watch(page)
    // Wide enough for the state to be written out in the top bar (below 1400 px it is a dot and a tooltip).
    await page.setViewportSize({ width: 1440, height: 900 })
    await simulatePicker(page, 'song-a')
    await page.goto('/app')
    await start(page)
    await expect(state(page)).toHaveText('Not saved')
    await expect(state(page)).toBeVisible()

    await addTrack(page, 'Drums')
    await expect(tracks(page)).toHaveCount(2)
    await expect(state(page)).toHaveText('Not saved')

    // The browser asks before the tab goes, while the project is in no folder.
    const probe = await context.newPage()
    await simulatePicker(probe, 'song-probe')
    await probe.goto('/app')
    await start(probe)
    await addTrack(probe, 'Drums')
    expect(await leaveAsks(probe)).toBe(true)

    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at \d{1,2}:\d{2}/)
    await expect(state(page)).toBeVisible()
    expect(await listFolder(page, 'song-a')).toEqual(expect.arrayContaining(['project.json', '.faulk/session.json', '.gitignore']))
    expect((await listFolder(page, 'song-a')).filter((p) => p.endsWith('track.json'))).toHaveLength(2)

    // An edit: unsaved for a moment, then saved by itself.
    await addTrack(page, 'Oscillator')
    await expect(tracks(page)).toHaveCount(3)
    await expect(state(page)).toHaveText('Unsaved changes')
    await expect(state(page)).toHaveText(/^Saved at/, { timeout: 8000 })
    expect((await listFolder(page, 'song-a')).filter((p) => p.endsWith('track.json'))).toHaveLength(3)

    // Reload: the project comes back by itself, with its three tracks.
    await page.reload()
    await expect(tracks(page)).toHaveCount(3)
    await expect(state(page)).toHaveText('Saved')
    await expect(page.getByTestId('project-name')).not.toHaveText('')
    await start(page)
    await expect(tracks(page)).toHaveCount(3)
    // And it goes on saving into the same folder.
    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText(/^Saved at/, { timeout: 8000 })
    expect((await listFolder(page, 'song-a')).filter((p) => p.endsWith('track.json'))).toHaveLength(4)
    expect(errors).toEqual([])
  })

  test('Ctrl+S saves now, and the menu says the same', async ({ page }) => {
    const { errors } = watch(page)
    await simulatePicker(page, 'song-keys')
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    await addTrack(page, 'Drums')
    await expect(state(page)).toHaveText('Unsaved changes')
    await page.keyboard.press('Control+s')
    await expect(state(page)).toHaveText(/^Saved at/)
    expect((await listFolder(page, 'song-keys')).filter((p) => p.endsWith('track.json'))).toHaveLength(2)
    expect(errors).toEqual([])
    // Everything is in the folder: closing the tab asks nothing.
    expect(await leaveAsks(page)).toBe(false)
  })

  test('a tab closed while an existing file is replaced leaves that file whole', async ({ page, context }) => {
    // The edit changes a file that exists (the track's header): the replacement, not a new file, is held open.
    await page.addInitScript(() => {
      Object.assign(window, { __faulkStoreInPage: true })
      const write = FileSystemWritableFileStream.prototype.write
      FileSystemWritableFileStream.prototype.write = function (...args: Parameters<typeof write>) {
        if (sessionStorage.getItem('hang')) {
          sessionStorage.setItem('hung', '1')
          return new Promise(() => undefined)
        }
        return write.apply(this, args)
      }
    })
    await simulatePicker(page, 'song-replace')
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    const trackFile = (await listFolder(page, 'song-replace')).find((p) => p.endsWith('track.json'))!
    const before = await readText(page, 'song-replace', trackFile)
    expect(before).toContain('"mute": false')

    await page.evaluate(() => sessionStorage.setItem('hang', '1'))
    await page.getByRole('button', { name: 'Mute Synth', exact: true }).click()
    await expect.poll(() => page.evaluate(() => sessionStorage.getItem('hung')), { timeout: 8000 }).toBe('1')
    const probe = await context.newPage()
    await page.close()

    await probe.goto('/')
    await expect.poll(() => probe.evaluate(async () => ((await navigator.locks.query()).held ?? []).length)).toBe(0)
    await simulatePicker(probe, 'song-replace')
    await probe.goto('/app')
    await expect(tracks(probe)).toHaveCount(1)
    expect(await readText(probe, 'song-replace', trackFile)).toBe(before)
    await expect(probe.getByRole('button', { name: 'Mute Synth', exact: true })).toHaveAttribute('aria-pressed', 'false')
  })

  test('Save as twice: the copy has its own identity, and nothing says it is open in another tab', async ({ page }) => {
    const { errors } = watch(page)
    await simulatePicker(page, 'song-first')
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    await addTrack(page, 'Drums')
    await page.evaluate(() => ((window as unknown as { __pick: string }).__pick = 'song-copy'))
    await saveAs(page)
    await expect(page.getByTestId('storage-notice')).toHaveCount(0)
    await expect(state(page)).toHaveText(/^Saved at/)
    const id = async (folder: string) => JSON.parse((await readText(page, folder, 'project.json'))!).id
    expect(await id('song-copy')).not.toBe(await id('song-first'))
    expect((await listFolder(page, 'song-copy')).filter((p) => p.endsWith('track.json'))).toHaveLength(2)
    // The first folder was left as it was: no copy of the second track.
    expect((await listFolder(page, 'song-first')).filter((p) => p.endsWith('track.json'))).toHaveLength(1)
    // And the page now saves into the copy.
    await addTrack(page, 'Oscillator')
    await expect(state(page)).toHaveText(/^Saved at/, { timeout: 8000 })
    expect((await listFolder(page, 'song-copy')).filter((p) => p.endsWith('track.json'))).toHaveLength(3)
    expect((await listFolder(page, 'song-first')).filter((p) => p.endsWith('track.json'))).toHaveLength(1)
    expect(errors).toEqual([])
  })

  test('a tab closed during a write leaves the old file intact, and the project opens as it was', async ({ page, context }) => {
    const { errors } = watch(page)
    // The store runs in the page, not in its Worker, so the test can hold a write open.
    await page.addInitScript(() => {
      Object.assign(window, { __faulkStoreInPage: true })
      const write = FileSystemWritableFileStream.prototype.write
      FileSystemWritableFileStream.prototype.write = function (...args: Parameters<typeof write>) {
        if (sessionStorage.getItem('hang')) {
          sessionStorage.setItem('hung', '1')
          return new Promise(() => undefined)
        }
        return write.apply(this, args)
      }
    })
    await simulatePicker(page, 'song-crash')
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    const before = await readText(page, 'song-crash', 'project.json')
    expect(before).toContain('"id"')

    await page.evaluate(() => sessionStorage.setItem('hang', '1'))
    await addTrack(page, 'Drums')
    await expect.poll(() => page.evaluate(() => sessionStorage.getItem('hung')), { timeout: 8000 }).toBe('1')
    await expect(state(page)).toHaveText('Saving…')
    // The tab goes away while the write is open.
    const probe = await context.newPage()
    await page.close()

    // The closed tab's lock on the project goes with it; wait for that, off the DAW's page.
    await probe.goto('/')
    await expect.poll(() => probe.evaluate(async () => ((await navigator.locks.query()).held ?? []).length)).toBe(0)
    await simulatePicker(probe, 'song-crash')
    const seen = watch(probe)
    await probe.goto('/app')
    await expect(tracks(probe), `${(await probe.getByTestId('storage-notice').allInnerTexts()).join(' ')} ${seen.errors.join(' ')}`).toHaveCount(1)
    expect(await readText(probe, 'song-crash', 'project.json')).toBe(before)
    expect((await listFolder(probe, 'song-crash')).filter((p) => p.endsWith('.crswap'))).toEqual([])
    expect(errors.filter((e) => !/closed/i.test(e))).toEqual([])
  })

  test('permission lost: a banner, a click, and saving resumes with nothing lost', async ({ page }) => {
    const { errors } = watch(page)
    await simulatePicker(page, 'song-perm')
    await simulatePermission(page)
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    const before = await readText(page, 'song-perm', 'project.json')

    await page.evaluate(() => ((window as unknown as { __perm: string }).__perm = 'prompt'))
    await addTrack(page, 'Drums')
    await expect(page.getByTestId('permission-banner')).toBeVisible({ timeout: 8000 })
    await expect(state(page)).toHaveText('Saving paused')
    expect(await readText(page, 'song-perm', 'project.json')).toBe(before)
    expect((await listFolder(page, 'song-perm')).filter((p) => p.endsWith('track.json'))).toHaveLength(1)

    // Denied once: still paused. Allowed: the banner goes and the track is written.
    await page.evaluate(() => ((window as unknown as { __allow: boolean }).__allow = false))
    await page.getByTestId('allow').click()
    await expect(page.getByTestId('permission-banner')).toBeVisible()
    await page.evaluate(() => ((window as unknown as { __allow: boolean }).__allow = true))
    await page.getByTestId('allow').click()
    await expect(page.getByTestId('permission-banner')).toHaveCount(0)
    await expect(state(page)).toHaveText(/^Saved at/, { timeout: 8000 })
    expect((await listFolder(page, 'song-perm')).filter((p) => p.endsWith('track.json'))).toHaveLength(2)
    expect(errors).toEqual([])
  })

  test('after a reload the browser asks again: a banner offers the project, and a click opens it', async ({ page }) => {
    const { errors } = watch(page)
    await simulatePicker(page, 'song-again')
    await simulatePermission(page)
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)

    await page.evaluate(() => sessionStorage.setItem('perm', 'prompt'))
    await page.reload()
    await expect(page.getByTestId('reopen-banner')).toBeVisible()
    // One clear main action (#474): it names the folder, and the start button gives up its violet while it waits.
    await expect(page.getByTestId('reopen')).toHaveText('Reopen “song-again”')
    await expect(page.getByTestId('engine-state').getByRole('button')).not.toHaveClass(/\bprimary\b/)
    await expect(page.locator('p.banner')).toHaveCount(1)
    await expect(tracks(page)).toHaveCount(0)
    await page.getByTestId('reopen').click()
    await expect(tracks(page)).toHaveCount(2)
    await expect(page.getByTestId('reopen-banner')).toHaveCount(0)
    expect(errors).toEqual([])
  })

  test('a folder the browser refuses, a cancelled dialog, and a folder that is not empty', async ({ page }) => {
    const { errors } = watch(page)
    await simulatePicker(page, 'song-full')
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')

    await page.evaluate(() => ((window as unknown as { __pickError: string }).__pickError = 'SecurityError'))
    await saveAs(page)
    await expect(page.getByTestId('storage-notice')).toContainText('did not allow that folder')
    await expect(state(page)).toHaveText('Not saved')

    await page.evaluate(() => ((window as unknown as { __pickError: string }).__pickError = 'AbortError'))
    await page.getByTestId('storage-notice').getByRole('button').click()
    await saveAs(page)
    await expect(page.getByTestId('storage-notice')).toHaveCount(0)

    await page.evaluate(async () => {
      ;(window as unknown as { __pickError?: string }).__pickError = undefined
      const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle('song-full', { create: true })
      const writable = await (await dir.getFileHandle('readme.txt', { create: true })).createWritable()
      await writable.write('other files live here')
      await writable.close()
    })
    await saveAs(page)
    await expect(page.getByTestId('storage-notice')).toContainText('not empty')
    expect(await listFolder(page, 'song-full')).toEqual(['readme.txt'])
    await expect(tracks(page)).toHaveCount(2)
    expect(errors).toEqual([])
  })

  test('a new project asks before it drops changes no folder holds, and writes the others first', async ({ page }) => {
    const { errors } = watch(page)
    await simulatePicker(page, 'song-new')
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')

    const newProject = async () => {
      await page.getByTestId('project-menu').click()
      await page.getByTestId('menu-new').click()
    }
    await newProject()
    await expect(page.getByTestId('confirm')).toBeVisible()
    await expect(page.getByTestId('confirm-cancel')).toBeFocused()
    await page.getByTestId('confirm-cancel').click()
    await expect(page.getByTestId('confirm')).toHaveCount(0)
    await expect(tracks(page)).toHaveCount(2)

    await newProject()
    await page.getByTestId('confirm-discard').click()
    await expect(tracks(page)).toHaveCount(1)
    await expect(state(page)).toHaveText('Not saved')

    // With a folder: the changes are written first, there is no question, and the folder is let go.
    await addTrack(page, 'Drums')
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    await addTrack(page, 'Oscillator')
    await newProject()
    await expect(page.getByTestId('confirm')).toHaveCount(0)
    await expect(tracks(page)).toHaveCount(1)
    await expect(state(page)).toHaveText('Not saved')
    expect((await listFolder(page, 'song-new')).filter((p) => p.endsWith('track.json'))).toHaveLength(3)
    // The new project does not write over the old folder.
    await addTrack(page, 'Drums')
    await page.waitForTimeout(3500)
    expect((await listFolder(page, 'song-new')).filter((p) => p.endsWith('track.json'))).toHaveLength(3)
    expect(errors).toEqual([])
  })

  test('a folder edited from outside is reloaded, or the user chooses when changes are unsaved', async ({ page }) => {
    const { errors } = watch(page)
    await simulatePicker(page, 'song-out')
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)
    const rename = (name: string) =>
      page.evaluate(async (title) => {
        const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle('song-out')
        const handle = await dir.getFileHandle('project.json')
        const project = JSON.parse(await (await handle.getFile()).text())
        project.name = title
        const writable = await handle.createWritable()
        await writable.write(JSON.stringify(project, null, 2) + '\n')
        await writable.close()
        window.dispatchEvent(new Event('focus'))
      }, name)

    // Nothing unsaved: the page reloads the folder and says so.
    await rename('Edited elsewhere')
    await expect(page.getByTestId('project-name')).toHaveText('Edited elsewhere', { timeout: 8000 })
    await expect(page.getByTestId('storage-notice')).toContainText('reloaded')

    // Unsaved changes: the user chooses; "Keep mine" writes over the folder.
    await page.getByTestId('storage-notice').getByRole('button').click()
    await addTrack(page, 'Drums')
    await rename('Edited again')
    await expect(page.getByTestId('external-banner')).toBeVisible({ timeout: 8000 })
    await expect(state(page)).toHaveText('Folder changed')
    await page.getByTestId('keep-mine').click()
    // Replacing the folder's files loses its outside edits: the page asks first, Cancel changes nothing.
    await expect(page.getByTestId('confirm')).toContainText('Replace the folder’s version')
    await page.getByTestId('confirm-cancel').click()
    await expect(page.getByTestId('external-banner')).toBeVisible()
    await page.getByTestId('keep-mine').click()
    await page.getByTestId('confirm-discard').click()
    await expect(page.getByTestId('external-banner')).toHaveCount(0)
    await expect(state(page)).toHaveText(/^Saved at/, { timeout: 8000 })
    expect(JSON.parse((await readText(page, 'song-out', 'project.json'))!).name).toBe('Edited elsewhere')
    expect((await listFolder(page, 'song-out')).filter((p) => p.endsWith('track.json'))).toHaveLength(2)

    // And "Load the folder's version" drops them.
    await addTrack(page, 'Oscillator')
    await rename('Third')
    await expect(page.getByTestId('external-banner')).toBeVisible({ timeout: 8000 })
    await page.getByTestId('load-disk').click()
    await expect(page.getByTestId('confirm')).toContainText('Load the folder’s version')
    await page.getByTestId('confirm-discard').click()
    await expect(page.getByTestId('project-name')).toHaveText('Third')
    await expect(tracks(page)).toHaveCount(2)
    expect(errors).toEqual([])
  })

  test('the same project in two tabs: the second does not take it', async ({ page, context }) => {
    await simulatePicker(page, 'song-two')
    await page.goto('/app')
    await start(page)
    await saveAs(page)
    await expect(state(page)).toHaveText(/^Saved at/)

    const second = await context.newPage()
    await simulatePicker(second, 'song-two')
    await second.goto('/app')
    await expect(second.getByTestId('storage-notice')).toContainText('open in another tab')
    await expect(tracks(second)).toHaveCount(0)
  })

  test('a browser without folders says what works, and still warns before the tab goes', async ({ page, context }) => {
    const { errors } = watch(page)
    await page.addInitScript(() => {
      Object.defineProperty(window, 'showDirectoryPicker', { value: undefined, configurable: true })
      ;(window as unknown as { __faulkNoBrowserStorage: boolean }).__faulkNoBrowserStorage = true
    })
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await page.getByTestId('project-menu').click()
    await expect(page.getByTestId('menu-unsupported')).toContainText('cannot save to a folder')
    await expect(page.getByTestId('menu-open')).toBeDisabled()
    await expect(page.getByTestId('menu-save-as')).toBeDisabled()
    await page.getByTestId('menu-save').click()
    await expect(page.getByTestId('storage-notice')).toContainText('cannot save to a folder')
    expect(context.pages()).toContain(page)
    expect(await leaveAsks(page)).toBe(true)
    expect(errors).toEqual([])
  })

  test('French: the state and the menu are in French', async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
    await simulatePicker(page, 'song-fr')
    await page.goto('/app')
    await start(page)
    await expect(state(page)).toHaveText('Non enregistré')
    await page.getByTestId('project-menu').click()
    await expect(page.getByTestId('menu-save-as')).toContainText('Enregistrer sous')
    await page.getByTestId('menu-save-as').click()
    await expect(state(page)).toHaveText(/^Enregistré à/)
  })
})
