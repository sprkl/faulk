import { expect, test, type Locator, type Page } from '@playwright/test'
import { engineUp, skipWithoutDevHooks, watch } from './support'

// #687: the light and the dark theme of the DAW. The project menu's Theme (System, Dark, Light) runs `view.set_theme`;
// the choice is kept by the browser and comes back on the next page before the first paint (the head's script); System
// follows the system's preference live; every canvas draws again in the new theme's tokens without a reload. No dev
// hook: the test runs on the production build too. `FAULK_E2E_SHOTS=<folder>` keeps captures of the main screens in
// both themes.

test.describe.configure({ mode: 'serial', timeout: 180_000 })

const SHOTS = process.env.FAULK_E2E_SHOTS

async function start(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
}

async function choose(page: Page, choice: 'system' | 'dark' | 'light') {
  await page.getByTestId('project-menu').click()
  await page.getByTestId(`menu-theme-${choice}`).click()
}

/** The mean luminance (0 to 255) of the bottom-right corner of a canvas: its background there. */
async function corner(canvas: Locator): Promise<number> {
  return canvas.evaluate((el) => {
    const c = el as HTMLCanvasElement
    const data = c.getContext('2d')!.getImageData(c.width - 24, c.height - 24, 20, 20).data
    let sum = 0
    for (let i = 0; i < data.length; i += 4) sum += 0.2126 * data[i]! + 0.7152 * data[i + 1]! + 0.0722 * data[i + 2]!
    return sum / (data.length / 4)
  })
}

const html = (page: Page) => page.locator('html')

/** Opens the piano roll on a new clip and clicks a few notes into it (a click on the roll adds one). */
async function rollWithNotes(page: Page) {
  await page.getByTestId('panel-piano-roll').click()
  await page.getByTestId('roll-create-clip').click()
  const roll = page.getByTestId('piano-roll')
  await expect(roll).toBeVisible()
  const box = (await page.getByTestId('roll-stage').boundingBox())!
  for (const [i, dy] of [0, -32, -16, 16, -48, 0].entries()) await page.mouse.click(box.x + 60 + i * 70, box.y + box.height / 2 + dy)
  await expect.poll(async () => Number(await roll.getAttribute('data-note-count'))).toBeGreaterThan(3)
}

test('the menu sets the theme, the browser keeps it, and the next page opens in it from its first paint', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  await expect(html(page)).toHaveAttribute('data-theme', 'dark')
  await page.getByTestId('project-menu').click()
  await expect(page.getByTestId('menu-theme-dark')).toHaveAttribute('aria-checked', 'true')
  await expect(page.getByTestId('menu-theme-system')).toHaveAttribute('aria-checked', 'false')
  await page.getByTestId('project-menu').click()
  await choose(page, 'light')
  await expect(html(page)).toHaveAttribute('data-theme', 'light')
  expect(await page.evaluate(() => localStorage.getItem('faulk.theme'))).toBe('light')
  const background = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor)
  expect(await background()).toBe('rgb(253, 252, 252)')

  // The theme the document had when the body began: set by the head's script, before the bundle ran.
  await page.addInitScript(() => {
    const w = window as unknown as { __themeAtBody?: string | null }
    new MutationObserver((_, observer) => {
      if (!document.body) return
      w.__themeAtBody = document.documentElement.getAttribute('data-theme')
      observer.disconnect()
      // Init scripts run before the parser makes `<html>`: the whole document is watched.
    }).observe(document, { childList: true, subtree: true })
  })
  await page.reload()
  expect(await page.evaluate(() => (window as unknown as { __themeAtBody?: string }).__themeAtBody)).toBe('light')
  await expect(html(page)).toHaveAttribute('data-theme', 'light')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  await choose(page, 'dark')
  await expect(html(page)).toHaveAttribute('data-theme', 'dark')
  expect(await background()).toBe('rgb(19, 18, 17)')
  expect(errors).toEqual([])
})

test('System follows the system preference, also when it changes during the session', async ({ page }) => {
  const { errors } = watch(page)
  await page.emulateMedia({ colorScheme: 'light' })
  await start(page)
  await choose(page, 'system')
  await expect(html(page)).toHaveAttribute('data-theme', 'light')
  await page.emulateMedia({ colorScheme: 'dark' })
  await expect(html(page)).toHaveAttribute('data-theme', 'dark')
  await page.emulateMedia({ colorScheme: 'light' })
  await expect(html(page)).toHaveAttribute('data-theme', 'light')
  // Another choice stops following it.
  await choose(page, 'dark')
  await page.emulateMedia({ colorScheme: 'light' })
  await expect(html(page)).toHaveAttribute('data-theme', 'dark')
  await choose(page, 'system')
  await expect(html(page)).toHaveAttribute('data-theme', 'light')
  expect(errors).toEqual([])
})

test('a second tab follows a theme chosen in the first (#698)', async ({ page, context }) => {
  const { errors } = watch(page)
  await start(page)
  const other = await context.newPage()
  const otherErrors = watch(other).errors
  await other.goto('/app')
  await other.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(other, 'Web')
  await expect(html(other)).toHaveAttribute('data-theme', 'dark')
  await choose(page, 'light')
  await expect(html(page)).toHaveAttribute('data-theme', 'light')
  // The storage event reaches the other tab, which applies the choice without writing it back.
  await expect(html(other)).toHaveAttribute('data-theme', 'light')
  expect(await other.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe('rgb(253, 252, 252)')
  await choose(other, 'dark')
  await expect(html(page)).toHaveAttribute('data-theme', 'dark')
  await other.close()
  expect(errors).toEqual([])
  expect(otherErrors).toEqual([])
})

test('a track colour of any hue reads in the clip grid in the light theme (#698)', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  await skipWithoutDevHooks(page)
  await page.evaluate(() => {
    const w = window as unknown as {
      __faulkRun: (c: unknown) => void
      __faulkView: () => { project: { tracks: Record<string, unknown>; scenes: Record<string, unknown> } }
    }
    w.__faulkRun({ type: 'scene.create' })
    const { tracks, scenes } = w.__faulkView().project
    const track = Object.keys(tracks)[0]!
    w.__faulkRun({ type: 'clip.create', track, at: { at: 'slot', scene: Object.keys(scenes)[0]! } })
    w.__faulkRun({ type: 'track.set_color', track, color: '#ffff00' })
  })
  await page.getByTestId('nav-grid').click()
  const cell = page.getByTestId('cell').first()
  await expect(cell).toBeVisible()
  await choose(page, 'light')
  await expect(html(page)).toHaveAttribute('data-theme', 'light')
  const ratioOf = () =>
    cell.evaluate((el) => {
      const lum = (css: string) => {
        const m = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(css)!
        const [r, g, b] = [m[1], m[2], m[3]].map((v) => {
          const x = Number(v) / 255
          return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4
        })
        return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!
      }
      const [hi, lo] = [lum(getComputedStyle(el).color), lum(getComputedStyle(document.body).backgroundColor)].sort((a, b) => b - a)
      return (hi! + 0.05) / (lo! + 0.05)
    })
  await expect.poll(ratioOf).toBeGreaterThanOrEqual(3)
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/light-grid-yellow-track.png` })
  // Dark keeps the hue as it is.
  await choose(page, 'dark')
  await expect.poll(() => cell.evaluate((el) => getComputedStyle(el).color)).toBe('rgb(255, 255, 0)')
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/dark-grid-yellow-track.png` })
  expect(errors).toEqual([])
})

test('the dialogs have rounded corners and the radio buttons the accent, in both themes (#698)', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  for (const choice of ['dark', 'light'] as const) {
    await choose(page, choice)
    await page.getByTestId('project-menu').click()
    await expect(page.getByTestId('menu-export')).toBeVisible()
    expect(parseFloat(await page.getByRole('menu').evaluate((el) => getComputedStyle(el).borderTopLeftRadius))).toBeGreaterThan(0)
    await page.getByTestId('menu-export').click()
    await expect(page.getByTestId('export-dialog')).toBeVisible()
    expect(parseFloat(await page.getByTestId('export-dialog').evaluate((el) => getComputedStyle(el).borderTopLeftRadius))).toBeGreaterThan(0)
    const accent = await page.getByTestId('export-song').evaluate((el) => {
      const probe = document.createElement('i')
      probe.style.color = getComputedStyle(document.documentElement).getPropertyValue('--fk-accent')
      document.body.append(probe)
      const expected = getComputedStyle(probe).color
      probe.remove()
      return { got: getComputedStyle(el).accentColor, expected }
    })
    expect(accent.got).toBe(accent.expected)
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/${choice}-export-dialog-radio.png` })
    await page.keyboard.press('Escape')
  }
  expect(errors).toEqual([])
})

test('every canvas draws again in the new theme, without a reload', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await start(page)
  await page.getByTestId('nav-arrangement').click()
  await rollWithNotes(page)
  // The arrangement's time grid, the piano roll's grid, its keyboard and its velocity lane.
  const canvases = page.locator('[data-testid=arr-stage] [data-testid=timegrid] canvas >> nth=0')
  const roll = page.getByTestId('piano-roll')
  const all = [canvases, roll.locator('[data-testid=timegrid] canvas').first(), roll.locator('.surface canvas')]
  await expect(roll).toBeVisible()
  const surfaces = await roll.locator('.surface canvas').count()
  expect(surfaces).toBeGreaterThan(0)
  const each = async () => {
    const out: number[] = []
    for (const c of all.slice(0, 2)) out.push(await corner(c))
    for (let i = 0; i < surfaces; i++) out.push(await corner(roll.locator('.surface canvas').nth(i)))
    return out
  }
  const dark = await each()
  for (const l of dark) expect(l).toBeLessThan(80)
  await choose(page, 'light')
  await expect.poll(async () => Math.min(...(await each())), { timeout: 5_000 }).toBeGreaterThan(180)
  await choose(page, 'dark')
  await expect.poll(async () => Math.max(...(await each())), { timeout: 5_000 }).toBeLessThan(80)
  expect(errors).toEqual([])
})

test('the main screens in both themes', async ({ page }) => {
  test.skip(!SHOTS, 'captures only with FAULK_E2E_SHOTS=<folder>')
  const { errors } = watch(page)
  for (const size of [
    { width: 1440, height: 900 },
    { width: 1024, height: 720 },
  ]) {
    await page.setViewportSize(size)
    if (size.width === 1440) {
      await start(page)
      await page.getByTestId('nav-arrangement').click()
      await rollWithNotes(page)
    }
    for (const choice of ['dark', 'light'] as const) {
      await choose(page, choice)
      const shot = async (name: string) => {
        // The view switch's pill slides: the capture waits for it to rest.
        await expect.poll(() => page.evaluate(() => document.getAnimations().filter((a) => a.playState === 'running').length)).toBe(0)
        await page.screenshot({ path: `${SHOTS}/${choice}-${size.width}-${name}.png` })
      }
      await page.getByTestId('nav-arrangement').click()
      await page.getByTestId('panel-devices').click()
      await shot('arrangement-devices')
      await page.getByTestId('panel-piano-roll').click()
      await shot('arrangement-piano-roll')
      await page.getByTestId('nav-grid').click()
      await shot('grid')
      await page.getByTestId('nav-mixer').click()
      await shot('mixer')
      await page.getByTestId('project-menu').click()
      await shot('project-menu')
      await page.keyboard.press('Escape')
      await page.getByTestId('project-menu').click()
      await page.getByTestId('menu-export').click()
      await expect(page.getByTestId('export-dialog')).toBeVisible()
      await shot('export-dialog')
      await page.keyboard.press('Escape')
      await page.getByTestId('nav-arrangement').click()
      await page.keyboard.press('Shift+Slash')
      await expect(page.getByRole('dialog')).toBeVisible()
      await shot('shortcuts-dialog')
      await page.keyboard.press('Escape')
    }
    await choose(page, 'dark')
  }
  expect(errors).toEqual([])
})
