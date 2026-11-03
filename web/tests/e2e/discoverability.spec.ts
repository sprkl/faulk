import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #474: the shortcut list ("?" button and key), the guide of the empty timeline, errors that say what to do
// (the tempo's range, an import that names the project and offers to open it), one banner at a time, and the
// project of last time back by itself after a reload.

test.use({ storageState: { cookies: [], origins: [] } })
test.describe.configure({ timeout: 90_000 })

const tracks = (page: Page) => page.getByTestId('track-header')
const dialog = (page: Page) => page.getByTestId('shortcuts-dialog')

async function start(page: Page) {
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

async function addTrack(page: Page, item: string) {
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

test.describe('the shortcut list', () => {
  test('the ? button is in the tab order, opens a modal dialog, traps the focus and gives it back on Escape', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    const opener = page.getByTestId('shortcuts-open')
    await expect(opener).toHaveAccessibleName('Keyboard shortcuts')
    await expect(dialog(page)).toBeHidden()
    await opener.focus()
    await page.keyboard.press('Enter')
    await expect(dialog(page)).toBeVisible()
    await expect(dialog(page)).toHaveAttribute('aria-modal', 'true')
    await expect(dialog(page)).toHaveAccessibleName('Keyboard shortcuts')
    // The focus moved inside on open, and Tab never puts it on the page behind (which is inert).
    expect(await page.evaluate(() => document.activeElement?.closest('dialog[open]') !== null)).toBe(true)
    for (let i = 0; i < 6; i++) {
      await page.keyboard.press('Tab')
      expect(await page.evaluate(() => document.activeElement?.closest('dialog[open]') !== null || document.activeElement === document.body)).toBe(true)
    }
    // Every line has its keys in the reading order of a screen reader.
    await expect(dialog(page).getByRole('heading', { level: 3 })).toHaveCount(7)
    await expect(dialog(page)).toContainText('Play and stop')
    await expect(dialog(page).getByRole('listitem').filter({ hasText: 'Undo' }).first()).toContainText('Ctrl+Z')
    await page.keyboard.press('Escape')
    await expect(dialog(page)).toBeHidden()
    await expect(opener).toBeFocused()
    expect(errors).toEqual([])
  })

  test('the ? key opens it and closes it, except in a text field; Alt+1 does nothing while it is open', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await page.keyboard.press('Shift+Slash')
    await expect(dialog(page)).toBeVisible()
    await page.keyboard.press('Alt+2')
    await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'arrangement')
    await page.keyboard.press('Shift+Slash')
    await expect(dialog(page)).toBeHidden()
    // With Ctrl or Alt it is no ? shortcut.
    await page.keyboard.press('Control+Shift+Slash')
    await page.keyboard.press('Alt+Shift+Slash')
    await expect(dialog(page)).toBeHidden()
    // The tempo field takes the character as text: no list.
    await page.getByTestId('transport-tempo').fill('')
    await page.keyboard.press('Shift+Slash')
    await expect(dialog(page)).toBeHidden()
  })

  test('the open list holds the keyboard: no undo and no note behind it', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(tracks(page)).toHaveCount(2)
    await page.keyboard.press('Shift+Slash')
    await expect(dialog(page)).toBeVisible()
    await page.keyboard.down('KeyA')
    await page.keyboard.press('Control+KeyZ')
    await page.keyboard.up('KeyA')
    // Space is the list's too: it does not play behind it.
    await page.keyboard.press('Space')
    await expect(page.getByTestId('transport-play')).toHaveAttribute('aria-pressed', 'false')
    await expect(tracks(page)).toHaveCount(2)
    await expect(page.getByTestId('voices')).toHaveText('Voices: 0')
    await page.keyboard.press('Escape')
    await expect(dialog(page)).toBeHidden()
    await page.getByTestId('transport-position').click()
    await page.keyboard.press('Control+KeyZ')
    await expect(tracks(page)).toHaveCount(1)
  })

  test('the empty timeline has a guide, and its button opens the list', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    const guide = page.getByTestId('arrangement-empty')
    await expect(guide).toContainText('Press Start audio')
    await expect(guide).toContainText('Add a track with + Add track')
    await start(page)
    // The project starts with a track and no clip: the guide goes on, one step shorter, without the start step.
    const noClips = page.getByTestId('arrangement-no-clips')
    await expect(noClips).toContainText('No clips yet')
    await expect(noClips).not.toContainText('Press Start audio')
    await expect(noClips).not.toContainText('Add a track with')
    await expect(noClips).toContainText('Press Space to play')
    await page.getByTestId('guide-shortcuts').click()
    await expect(dialog(page)).toBeVisible()
    expect(await page.evaluate(() => document.activeElement?.closest('dialog[open]') !== null)).toBe(true)
    await page.keyboard.press('Escape')
    await expect(page.getByTestId('guide-shortcuts')).toBeFocused()
    // It goes when the timeline has a clip: a double-click on a lane makes one.
    const stage = (await page.getByTestId('arr-stage').boundingBox())!
    await page.mouse.dblclick(stage.x + 400, stage.y + 28 + 22)
    await expect(page.getByTestId('arrangement-no-clips')).toHaveCount(0)
    await expect(page.getByTestId('arrangement-empty')).toHaveCount(0)
  })

  test('French: the button, the dialog and the guide are in French', async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await expect(page.getByTestId('shortcuts-open')).toHaveAccessibleName('Raccourcis clavier')
    await expect(page.getByTestId('arrangement-empty')).toContainText('Appuie sur Démarrer l’audio')
    await page.getByTestId('shortcuts-open').click()
    await expect(dialog(page)).toContainText('Lecture et arrêt')
  })

  test('the tooltips carry the shortcuts', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await expect(page.getByTestId('transport-play')).toHaveAttribute('title', 'Play (Space)')
    await expect(page.getByTestId('nav-mixer')).toHaveAttribute('title', /Alt\+3/)
    await page.getByTestId('project-menu').click()
    await expect(page.getByTestId('menu-save')).toContainText('Ctrl+S')
  })
})

test.describe('errors that say what to do', () => {
  test('a tempo out of range says the range, keeps the text to fix, and Escape puts the tempo back', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    const tempo = page.getByTestId('transport-tempo')
    await expect(tempo).toHaveValue('120')
    await tempo.fill('5000')
    await tempo.press('Enter')
    await expect(page.getByTestId('tempo-error')).toHaveText('Tempo goes from 20 to 999 BPM. Type a number in that range.')
    await expect(tempo).toHaveAttribute('aria-invalid', 'true')
    await expect(tempo).toHaveValue('5000')
    await expect(tempo).toBeFocused()
    // A valid value clears it and is applied.
    await tempo.fill('90')
    await tempo.press('Enter')
    await expect(page.getByTestId('tempo-error')).toHaveCount(0)
    await expect(tempo).toHaveAttribute('aria-invalid', 'false')
    // Leaving with text that is no tempo puts the project's tempo back and still says why.
    await tempo.fill('fast')
    await page.getByTestId('transport-position').click()
    await expect(tempo).toHaveValue('90')
    await expect(page.getByTestId('tempo-error')).toBeVisible()
    await tempo.focus()
    await tempo.fill('0')
    await tempo.press('Escape')
    await expect(page.getByTestId('tempo-error')).toHaveCount(0)
    await expect(tempo).toHaveValue('90')
  })

  test('importing a project that is already kept names it and offers to open it; one banner at a time', async ({ page }, testInfo) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(tracks(page)).toHaveCount(2)
    await expect(page.getByTestId('save-state')).toHaveText(/^Draft saved at/, { timeout: 20_000 })
    // The draft's name (#473: the moment it started).
    const name = ((await page.getByTestId('project-name').textContent()) ?? '').trim()
    expect(name).toMatch(/^Sketch /)
    await page.getByTestId('project-menu').click()
    const [download] = await Promise.all([page.waitForEvent('download'), page.getByTestId('menu-export-pack').click()])
    const pack = testInfo.outputPath('again.faulkpack')
    await download.saveAs(pack)

    // The project open: the notice names it and has nothing to open.
    await page.getByTestId('project-menu').click()
    let [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.getByTestId('menu-import-pack').click()])
    await chooser.setFiles(pack)
    await expect(page.getByTestId('storage-notice')).toContainText('is already open')
    await expect(page.getByTestId('storage-notice')).toContainText(name)
    await expect(page.getByTestId('notice-open')).toHaveCount(0)
    await page.getByTestId('storage-notice').getByRole('button', { name: 'Dismiss' }).click()

    // Another project open: the same file names the one kept in the browser, and the button opens it.
    await page.getByTestId('project-menu').click()
    await page.getByTestId('menu-new').click()
    await expect(tracks(page)).toHaveCount(1)
    await page.getByTestId('project-menu').click()
    ;[chooser] = await Promise.all([page.waitForEvent('filechooser'), page.getByTestId('menu-import-pack').click()])
    await chooser.setFiles(pack)
    const notice = page.getByTestId('storage-notice')
    await expect(notice).toContainText('already kept in this browser')
    await expect(page.locator('p.banner')).toHaveCount(1)
    await expect(page.getByTestId('notice-open')).toHaveText(`Open “${name}”`)
    await page.getByTestId('notice-open').click()
    await expect(tracks(page)).toHaveCount(2)
    await expect(notice).toHaveCount(0)
    expect(errors).toEqual([])
  })
})

test.describe('after a reload', () => {
  test('the project of last time is back by itself, with no banner and no extra violet button', async ({ page }) => {
    const { errors } = watch(page)
    await page.setViewportSize({ width: 1440, height: 900 })
    await page.goto('/app')
    await start(page)
    await addTrack(page, 'Drums')
    await expect(page.getByTestId('save-state')).toHaveText(/^Draft saved at/, { timeout: 20_000 })
    await page.reload()
    await expect(tracks(page)).toHaveCount(2)
    await expect(page.getByTestId('reopen-banner')).toHaveCount(0)
    await expect(page.locator('p.banner')).toHaveCount(0)
    // Before the audio, one main action: the start button.
    const buttons = page.locator('button')
    const violet = await buttons.evaluateAll((all) =>
      all.filter((b) => getComputedStyle(b).backgroundColor === 'rgb(102, 51, 244)').map((b) => b.textContent?.trim()),
    )
    expect(violet).toEqual(['Start audio'])
    expect(errors).toEqual([])
  })
})
