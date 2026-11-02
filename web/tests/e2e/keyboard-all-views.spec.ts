import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// #361: the computer keyboard plays the selected track in every view, not only the instrument view.
// Real worklet, engine and project core; the per-track peaks of the hidden `feedback` element say that
// the track sounds. The keys are shared in the order of `playsNote` (`src/lib/keyboard.ts`): a text field
// first, a control the keyboard focused, a menu or dialog, then a view's shortcuts, then the notes.

const trackPeak = async (page: Page) => Number(((await page.getByTestId('feedback').getAttribute('data-track-peaks')) ?? '').split(',')[0])
const masterPeak = async (page: Page) => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))

async function start(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('engine-mode')).toBeVisible()
  await expect(page.locator('[data-testid=track-header][aria-current=true]')).toBeVisible()
}

/** A, held a moment: the selected track sounds, then it falls silent. */
async function playsA(page: Page, what: string) {
  await expect.poll(() => masterPeak(page), { timeout: 5000, message: `${what}: silent before` }).toBeLessThan(0.001)
  await page.keyboard.down('KeyA')
  await expect.poll(() => trackPeak(page), { message: `${what}: A sounds` }).toBeGreaterThan(0.05)
  await expect(page.getByTestId('voices')).toContainText('1')
  await page.keyboard.up('KeyA')
  await expect.poll(() => masterPeak(page), { timeout: 5000, message: `${what}: silent after` }).toBeLessThan(0.001)
}

/** A key that must play nothing, pressed for as long as a note would take to sound. */
async function silent(page: Page, what: string, press: () => Promise<void>) {
  await press()
  await page.waitForTimeout(400)
  expect(await trackPeak(page), `${what}: no note`).toBeLessThan(0.001)
}

test('the keyboard plays in the arrangement, grid and mixer views and with the roll and devices panels', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  for (const view of ['arrangement', 'grid', 'mixer']) {
    await page.getByTestId(`nav-${view}`).click()
    await expect(page.getByTestId('view')).toHaveAttribute('data-view', view)
    await playsA(page, view)
  }
  for (const panel of ['piano-roll', 'devices']) {
    await page.getByTestId(`panel-${panel}`).click()
    await expect(page.getByTestId('bottom-panel')).toBeVisible()
    await playsA(page, `panel ${panel}`)
  }
  expect(errors.filter((e) => !/helper|WebSocket|ERR_CONNECTION/i.test(e))).toEqual([])
})

test('Z and X step the octave, shown in the top bar and in the keyboard tab, and the panic button stops the notes', async ({ page }) => {
  await start(page)
  await page.getByTestId('nav-arrangement').click()
  await page.getByTestId('panel-keyboard').click()
  const octave = page.getByTestId('keys-octave')
  const bar = page.getByTestId('keys-octave-bar')
  await expect(octave).toHaveText('Octave 4')
  await expect(bar).toHaveText('Oct 4')
  await page.keyboard.press('KeyX')
  await expect(octave).toHaveText('Octave 5')
  await expect(bar).toHaveText('Oct 5')
  await page.keyboard.press('KeyZ')
  await page.keyboard.press('KeyZ')
  await expect(octave).toHaveText('Octave 3')
  await page.keyboard.down('KeyA')
  await expect.poll(() => trackPeak(page)).toBeGreaterThan(0.05)
  await page.getByTestId('panic').click()
  await expect.poll(() => masterPeak(page), { timeout: 5000 }).toBeLessThan(0.001)
  await page.keyboard.up('KeyA')
})

test('a text field, a keyboard-focused control and a modifier keep their keys', async ({ page }) => {
  await start(page)
  await page.getByTestId('nav-arrangement').click()
  // 1. The tempo field is a text field.
  await silent(page, 'tempo field', async () => {
    await page.getByTestId('transport-tempo').focus()
    await page.keyboard.down('KeyA')
    await page.keyboard.up('KeyA')
  })
  // 2. A button the keyboard focused (Tab) keeps its keys; the one the mouse clicked does not.
  await silent(page, 'keyboard-focused button', async () => {
    await page.getByTestId('nav-grid').focus()
    await page.keyboard.press('Tab')
    await page.keyboard.press('Shift+Tab')
    await page.keyboard.down('KeyA')
    await page.keyboard.up('KeyA')
  })
  await page.getByTestId('nav-mixer').click()
  await playsA(page, 'button clicked with the mouse')
  // 3. Ctrl + A is a shortcut, not a note.
  await silent(page, 'Ctrl + A', async () => {
    await page.keyboard.press('Control+KeyA')
  })
})

test('an open menu keeps its keys, and a window that loses the focus ends the held notes', async ({ page }) => {
  await start(page)
  await page.getByTestId('nav-grid').click()
  await page.getByTestId('project-menu').click()
  await expect(page.getByRole('menu')).toBeVisible()
  await silent(page, 'project menu', async () => {
    await page.keyboard.down('KeyA')
    await page.keyboard.up('KeyA')
  })
  await page.keyboard.press('Escape')
  await page.keyboard.down('KeyA')
  await expect(page.getByTestId('voices')).toHaveText('Voices: 1')
  await page.evaluate(() => window.dispatchEvent(new Event('blur')))
  await expect(page.getByTestId('voices')).toHaveText('Voices: 0')
  await page.keyboard.up('KeyA')
})

test('the roll and the grid keep their shortcuts while the keyboard plays', async ({ page }) => {
  await start(page)
  await page.getByTestId('panel-piano-roll').click()
  await expect(page.getByTestId('bottom-panel')).toBeVisible()
  // The keys of the roll (arrows, Delete, F2) play nothing and a note key still plays with its grid focused.
  const roll = page.getByTestId('bottom-panel').locator('[tabindex="0"]').first()
  await roll.focus()
  await silent(page, 'roll arrows', async () => {
    await page.keyboard.press('ArrowUp')
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('Delete')
  })
  await playsA(page, 'roll focused')
  await page.getByTestId('nav-arrangement').click()
  await page.locator('#arr-stage-grid').focus()
  await page.keyboard.press('Home')
  await silent(page, 'arrangement Home', async () => {
    await page.keyboard.press('ArrowRight')
  })
  await playsA(page, 'timeline focused')
})
