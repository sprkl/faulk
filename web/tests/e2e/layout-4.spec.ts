import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #472: layout by screen and phase. The top bar stays on one row from 1280 px in every view and both languages, the engine
// pill is a button that opens the engine dialog, the keyboard is a tab of the bottom panel, the mixer takes the whole height
// with its master always in sight and the panel folded, the panel maximises from the keyboard (Alt+M), the views switch with
// Alt+1/2/3, and an Echo shows its note value when synced. Real browser, real engine.

const SIZES = [
  { width: 1920, height: 1080 },
  { width: 1440, height: 900 },
  { width: 1280, height: 800 },
]

async function start(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await expect(page.locator('[data-testid=track-header][aria-current=true]')).toBeVisible()
}

/** The bar is one row: its height is the minimum, every child sits inside it, and the page does not scroll sideways. */
async function expectOneRow(page: Page, what: string) {
  const geometry = await page.evaluate(() => {
    const bar = document.querySelector('header.top')!.getBoundingClientRect()
    const outside = [...document.querySelectorAll('header.top > *')]
      .filter((el) => el.getBoundingClientRect().width > 1)
      .filter((el) => {
        const r = el.getBoundingClientRect()
        return r.top < bar.top - 1 || r.bottom > bar.bottom + 1 || r.right > bar.right + 1
      })
      .map((el) => el.className || el.tagName)
    const meter = document.querySelector('[data-testid=top-meter]')!.getBoundingClientRect()
    return { height: bar.height, outside, scroll: document.documentElement.scrollWidth - document.documentElement.clientWidth, meter: meter.width }
  })
  expect(geometry.height, `${what}: one row`).toBeLessThanOrEqual(56)
  expect(geometry.outside, `${what}: nothing leaves the bar`).toEqual([])
  expect(geometry.scroll, `${what}: no horizontal scroll`).toBeLessThanOrEqual(0)
  expect(geometry.meter, `${what}: the master meter is readable`).toBeGreaterThanOrEqual(60)
}

for (const size of SIZES) {
  for (const lang of ['en', 'fr'] as const) {
    test(`the top bar is one row in every view at ${size.width}x${size.height}, ${lang}, with a long project name`, async ({ page }) => {
      const { errors } = watch(page)
      await page.setViewportSize(size)
      await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
      await start(page)
      await page.evaluate(() =>
        (window as unknown as { __faulkRun: (c: unknown) => void }).__faulkRun({
          type: 'project.rename',
          name: 'A very long project name for the layout test',
        }),
      )
      for (const view of ['arrangement', 'grid', 'mixer']) {
        await page.getByTestId(`nav-${view}`).click()
        await expect(page.getByTestId('view')).toHaveAttribute('data-view', view)
        await expectOneRow(page, `${view} ${lang}`)
        const pill = (await page.getByTestId('engine-mode').boundingBox())!
        const bar = (await page.locator('header.top').boundingBox())!
        expect(pill.y, 'the engine pill is on the first row').toBeGreaterThanOrEqual(bar.y)
        expect(pill.y + pill.height).toBeLessThanOrEqual(bar.y + bar.height)
      }
      // The mixer master is whole on screen, and so is the arrangement's Master lane.
      await page.getByTestId('nav-mixer').click()
      const master = (await page.getByTestId('mixer-master').boundingBox())!
      expect(master.y + master.height).toBeLessThanOrEqual(size.height)
      expect(master.x + master.width).toBeLessThanOrEqual(size.width)
      await page.getByTestId('nav-arrangement').click()
      await expect(page.getByText(/^master$/i).first()).toBeInViewport()
      expect(errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
    })
  }
}

test('the engine pill is a button that opens an accessible dialog with the audio, the helper and the MIDI, and gives the focus back', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  const pill = page.getByTestId('engine-mode')
  await expect(pill).toHaveJSProperty('tagName', 'BUTTON')
  await expect(pill).toHaveAttribute('aria-haspopup', 'dialog')
  await expect(page.getByTestId('latency')).toContainText('~')
  await pill.focus()
  await page.keyboard.press('Enter')
  const dialog = page.getByRole('dialog', { name: 'Audio engine and MIDI' })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('heading', { name: 'Audio', exact: true })).toBeVisible()
  await expect(dialog.getByRole('heading', { name: 'Native helper' })).toBeVisible()
  await expect(dialog.getByRole('heading', { name: 'MIDI input' })).toBeVisible()
  await expect(page.getByTestId('engine-dialog-mode')).toContainText('Web')
  await expect(page.getByTestId('engine-dialog-load')).toBeVisible()
  // The keys belong to the dialog, even when a click on its text leaves the focus on the body: no note, no view switch, no panel change.
  await dialog.getByRole('heading', { name: 'Audio', exact: true }).click()
  await page.evaluate(() => (document.activeElement as HTMLElement).blur())
  await page.keyboard.down('KeyA')
  await page.waitForTimeout(300)
  await expect(page.getByTestId('voices')).toHaveText('Voices: 0')
  await page.keyboard.up('KeyA')
  await page.keyboard.press('Alt+3')
  await page.keyboard.press('Alt+M')
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'grid')
  await expect(page.getByTestId('bottom-panel-maximize')).toHaveAttribute('aria-pressed', 'false')
  await page.keyboard.press('Escape')
  await expect(dialog).toBeHidden()
  await expect(pill).toBeFocused()
  // All notes off is in the dialog too: it releases a note that sounds (a held key whose key-up the page never gets).
  await page.getByTestId('engine-state').focus()
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.down('KeyA')
  await expect(page.getByTestId('voices')).toHaveText('Voices: 1')
  await pill.click()
  await page.getByTestId('panic-dialog').click()
  await expect(page.getByTestId('voices')).toHaveText('Voices: 0')
  await page.keyboard.press('Escape')
  await page.keyboard.up('KeyA')
  expect(errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})

test('the language is switched from the project menu, in both directions, and kept reachable', async ({ page }) => {
  await start(page)
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-lang-fr').click()
  await expect(page.getByTestId('nav-mixer')).toHaveText('Mixeur')
  await page.getByTestId('project-menu').click()
  await expect(page.getByTestId('menu-lang-fr')).toHaveAttribute('aria-checked', 'true')
  await page.getByTestId('menu-lang-en').click()
  await expect(page.getByTestId('nav-mixer')).toHaveText('Mixer')
})

test('the on-screen keyboard is a tab of the bottom panel next to the piano roll, with the octave and all notes off', async ({ page }) => {
  await start(page)
  const tabs = page.getByTestId('bottom-panel').locator('.tabs button')
  await expect(tabs).toHaveText(['Devices', 'History', 'Keyboard', 'Piano roll', '', ''])
  await page.getByTestId('panel-keyboard').click()
  await expect(page.getByTestId('keyboard-panel')).toBeVisible()
  await expect(page.getByTestId('keys-octave')).toHaveText('Octave 4')
  await page.keyboard.press('KeyX')
  await expect(page.getByTestId('keys-octave')).toHaveText('Octave 5')
  await expect(page.getByTestId('keys-octave-bar')).toHaveText('Oct 5')
  await page.keyboard.down('KeyA')
  await expect(page.getByTestId('voices')).toHaveText('Voices: 1')
  await page.getByTestId('panic').click()
  await expect(page.getByTestId('voices')).toHaveText('Voices: 0')
  await page.keyboard.up('KeyA')
  await expect(page.getByTestId('nav-play')).toHaveCount(0)
})

test('Alt+1, Alt+2 and Alt+3 switch the views, listed in their tooltips, and do nothing in a text field', async ({ page }) => {
  await start(page)
  await expect(page.getByTestId('nav-arrangement')).toHaveAttribute('title', 'Arrangement (Alt+1)')
  await expect(page.getByTestId('nav-mixer')).toHaveAttribute('title', 'Mixer (Alt+3)')
  await page.keyboard.press('Alt+3')
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'mixer')
  await page.keyboard.press('Alt+2')
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'grid')
  await page.keyboard.press('Alt+1')
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'arrangement')
  // No note is played by these keys.
  await expect(page.getByTestId('voices')).toHaveText('Voices: 0')
  // In the tempo field the keys stay the field's.
  await page.getByRole('textbox', { name: /tempo/i }).first().focus()
  await page.keyboard.press('Alt+3')
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', 'arrangement')
})

test('the mixer folds the bottom panel and gives it back, keeps its master in sight with many tracks, and adds a track from the ghost strip', async ({
  page,
}) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1280, height: 800 })
  await start(page)
  await page.getByTestId('nav-arrangement').click()
  const panel = page.getByTestId('bottom-panel')
  await expect(panel).toHaveAttribute('data-collapsed', 'false')
  await page.getByTestId('nav-mixer').click()
  await expect(panel).toHaveAttribute('data-collapsed', 'true')
  // The ghost strip adds a MIDI track through the same command as the track list.
  for (let i = 0; i < 12; i++) {
    await page.getByTestId('mixer-add-track').click()
    await page.getByTestId('mixer-add-track-menu').getByRole('menuitem').first().click()
  }
  await expect(page.getByTestId('mixer-strip')).toHaveCount(13)
  // Master pinned: whole on screen with 13 strips to its left, which scroll.
  const master = (await page.getByTestId('mixer-master').boundingBox())!
  expect(master.x + master.width).toBeLessThanOrEqual(1280)
  expect(master.y + master.height).toBeLessThanOrEqual(800)
  const strip = (await page.getByTestId('mixer-strip').first().boundingBox())!
  expect(strip.width).toBeGreaterThan(100)
  expect(strip.width).toBeLessThan(130)
  // The strips are as tall as the main area, not the 400 px of their content.
  const view = (await page.getByTestId('view').boundingBox())!
  expect(strip.height).toBeGreaterThan(view.height - 60)
  // A strip's Devices button unfolds the panel for this visit and moves the focus into it.
  await page.getByTestId('mixer-devices').first().click()
  await expect(panel).toHaveAttribute('data-collapsed', 'false')
  await expect(page.getByTestId('devices')).toBeVisible()
  await expect(page.getByTestId('bottom-panel-body')).toBeFocused()
  // Leaving the mixer: the panel is as the user left it (unfolded); folding it by hand is kept.
  await page.getByTestId('nav-arrangement').click()
  await expect(panel).toHaveAttribute('data-collapsed', 'false')
  await page.getByTestId('bottom-panel-fold').click()
  await expect(panel).toHaveAttribute('data-collapsed', 'true')
  await page.getByTestId('nav-mixer').click()
  await page.getByTestId('nav-grid').click()
  await expect(panel).toHaveAttribute('data-collapsed', 'true')
  await page.getByTestId('bottom-panel-fold').click()
  await expect(panel).toHaveAttribute('data-collapsed', 'false')
  expect(errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})

test('Alt+M maximises the bottom panel and restores it, the button says the key, and resizing ends it', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  const panel = page.getByTestId('bottom-panel')
  const height = async () => Math.round((await panel.boundingBox())!.height)
  const before = await height()
  await expect(page.getByTestId('bottom-panel-maximize')).toHaveAttribute('title', /Alt\+M/)
  await page.keyboard.press('Alt+M')
  await expect(page.getByTestId('bottom-panel-maximize')).toHaveAttribute('aria-pressed', 'true')
  await expect.poll(height).toBeGreaterThan(before + 150)
  await page.keyboard.press('Alt+M')
  await expect.poll(height).toBe(before)
  await page.getByTestId('bottom-panel-maximize').click()
  await expect.poll(height).toBeGreaterThan(before + 150)
  await page.getByTestId('bottom-panel-handle').focus()
  await page.keyboard.press('ArrowDown')
  await expect(page.getByTestId('bottom-panel-maximize')).toHaveAttribute('aria-pressed', 'false')
})

test('the piano roll and the devices sit side by side when the panel is wide, and an Echo shows its note value when synced', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('roll-devices')).toBeVisible()
  await expect(page.getByTestId('roll-devices').getByTestId('device-card').first()).toBeVisible()
  await page.getByTestId('panel-devices').click()
  await expect(page.getByTestId('roll-devices')).toHaveCount(0)
  await page.getByTestId('device-browser').getByRole('button', { name: 'Echo' }).click()
  const echo = page.locator('[data-testid=device-card][data-kind="faulk.echo"]')
  await expect(echo).toBeVisible()
  // Maximised, the panel shows every control of the card: nothing is cut by the panel's scroll.
  await page.keyboard.press('Alt+M')
  await expect(echo.getByTestId('param-feedback')).toBeInViewport()
  await expect(echo.getByTestId('param-mix')).toBeInViewport()
  // Synced is the default: the note value shows, the free time does not.
  await expect(echo.getByTestId('param-division')).toBeVisible()
  await expect(echo.getByTestId('param-time')).toHaveCount(0)
  await echo.getByRole('radio', { name: 'Free' }).click()
  await expect(echo.getByTestId('param-time')).toBeVisible()
  await expect(echo.getByTestId('param-division')).toHaveCount(0)
})

test('the Drums card shows all its knobs without a scroll in a maximised panel at 1440 px', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  await page.getByTestId('panel-devices').click()
  await page.getByTestId('device-browser').getByRole('button', { name: 'Drums' }).click()
  const drums = page.locator('[data-testid=device-card][data-kind="faulk.drums"]')
  await expect(drums).toBeVisible()
  await page.keyboard.press('Alt+M')
  const knobs = drums.locator('[data-testid^=param-]')
  const count = await knobs.count()
  expect(count).toBeGreaterThanOrEqual(16)
  for (let i = 0; i < count; i++) await expect(knobs.nth(i)).toBeInViewport()
  const chain = page.getByTestId('device-chain')
  expect(await chain.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1)
})
