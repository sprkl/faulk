import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #878: the top bar at the narrowest supported width (800 px: no document gives one, the issue names it) keeps the engine pill, the tempo and the
// transport in sight and inside the viewport; what does not fit goes into the bar's more menu, usable from the keyboard.

async function open(page: Page, width: number, lang: 'en' | 'fr' = 'en') {
  await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
  await page.setViewportSize({ width, height: 900 })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
  await expect(page.getByTestId('transport-tempo')).toBeEnabled()
}

/** Every control of the bar that must stay: in the viewport, not cut by the bar's right edge. */
const ALWAYS = [
  'nav-arrangement',
  'nav-grid',
  'nav-mixer',
  'project-menu',
  'transport-rewind',
  'transport-play',
  'transport-stop',
  'transport-record',
  'transport-tempo',
  'engine-mode',
  'shortcuts-open',
]

async function assertInside(page: Page, width: number) {
  await expect(async () => {
    const out = await page.evaluate((ids) => {
      const bar = document.querySelector('header.top')!
      const b = bar.getBoundingClientRect()
      return {
        overflow: bar.scrollWidth - bar.clientWidth,
        docOverflow: document.documentElement.scrollWidth - window.innerWidth,
        boxes: ids.map((id) => {
          const r = document.querySelector(`[data-testid="${id}"]`)?.getBoundingClientRect()
          return { id, w: r?.width ?? 0, x: r?.left ?? -1, r: r?.right ?? -1, barR: b.right }
        }),
      }
    }, ALWAYS)
    expect(out.overflow, 'the bar scrolls sideways').toBeLessThanOrEqual(0)
    expect(out.docOverflow, 'the page scrolls sideways').toBeLessThanOrEqual(0)
    for (const c of out.boxes) {
      expect(c.w, `${c.id} is not painted`).toBeGreaterThan(0)
      expect(c.x, `${c.id} left of the viewport`).toBeGreaterThanOrEqual(-0.5)
      expect(c.r, `${c.id} right of the viewport (${width})`).toBeLessThanOrEqual(width + 0.5)
      expect(c.r, `${c.id} right of the bar`).toBeLessThanOrEqual(c.barR + 0.5)
    }
  }).toPass({ timeout: 3000 })
  for (const id of ['engine-mode', 'transport-tempo', 'transport-play']) await expect(page.getByTestId(id)).toBeInViewport({ ratio: 1 })
}

for (const [lang, width] of [
  ['en', 800],
  ['fr', 800],
  ['en', 1100],
  ['fr', 1100],
  ['en', 1280],
  ['fr', 1280],
] as const)
  test(`the engine pill, the tempo and the transport stay in the top bar at ${width} px in ${lang === 'en' ? 'English' : 'French'} (#878)`, async ({
    page,
  }) => {
    const { errors } = watch(page)
    await open(page, width, lang)
    await assertInside(page, width)
    await expect(page.getByTestId('engine-mode')).toBeVisible()
    await expect(page.getByTestId('transport-tempo')).toBeVisible()
    expect(errors).toEqual([])
  })

test('at 800 px the bar still fits when the pill is at its widest (#878)', async ({ page }) => {
  const { errors } = watch(page)
  await open(page, 800)
  // 20 px more in the pill: a 100 % load and a three-digit latency are wider than the idle engine's.
  await page.evaluate(() => {
    const el = document.createElement('span')
    el.style.cssText = 'display:inline-block;flex:none;height:28px;width:20px'
    document.querySelector('[data-testid=engine-mode]')!.append(el)
  })
  await assertInside(page, 800)
  expect(errors).toEqual([])
})

test('at 800 px the more menu holds what the bar folded and works from the keyboard (#878)', async ({ page }) => {
  const { errors } = watch(page)
  await open(page, 800)
  const folded = ['transport-tap', 'transport-numerator', 'transport-loop', 'transport-metronome', 'top-meter']
  for (const id of folded) await expect(page.getByTestId(id)).toBeHidden()
  const more = page.getByTestId('bar-more')
  await expect(more).toBeVisible()
  await expect(more).toHaveAttribute('aria-haspopup', 'menu')
  await expect(more).toHaveAttribute('aria-expanded', 'false')
  await more.focus()
  await page.keyboard.press('Enter')
  const menu = page.getByTestId('context-menu')
  await expect(menu).toBeVisible()
  await expect(menu).toHaveAttribute('role', 'menu')
  await expect(more).toHaveAttribute('aria-expanded', 'true')
  await expect(page.getByTestId('ctx-more-tap')).toContainText('Alt+T')
  await expect(page.getByTestId('ctx-more-signature-4-4')).toContainText('✓')
  // The arrows move through the entries, Enter runs one: the time signature is a command.
  await page.keyboard.press('ArrowDown')
  await expect(menu.locator('[role=menuitem]:focus').first()).toBeVisible()
  await page.getByTestId('ctx-more-signature-3-4').focus()
  await page.keyboard.press('Enter')
  await expect(menu).toHaveCount(0)
  await more.click()
  await expect(page.getByTestId('ctx-more-signature-3-4')).toContainText('✓')
  // The toggles run the bar's own commands: the loop (a region is made when the project has none) and the metronome.
  await expect(page.getByTestId('ctx-more-loop')).not.toContainText('✓')
  await page.getByTestId('ctx-more-loop').click()
  await more.click()
  await expect(page.getByTestId('ctx-more-loop')).toContainText('✓')
  await page.getByTestId('ctx-more-metronome').click()
  await more.click()
  await expect(page.getByTestId('ctx-more-metronome')).toContainText('✓')
  // Escape closes the menu and gives the focus back to the button.
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)
  await expect(more).toBeFocused()
  await expect(more).toHaveAttribute('aria-expanded', 'false')

  // With room again, the controls are back in the bar, with what the menu set, and the button is gone.
  await more.blur()
  await page.setViewportSize({ width: 1920, height: 900 })
  await expect(more).toBeHidden()
  for (const id of folded) await expect(page.getByTestId(id)).toBeVisible()
  await expect(page.getByTestId('transport-loop')).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('transport-metronome')).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByTestId('transport-numerator')).toHaveValue('3')
  expect(errors).toEqual([])
})
