import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// UI-1 (#105): the app shell's navigation in a real browser. The views to visit come from the
// registry's source (`src/views/registry.ts`, parsed because Playwright cannot import the .svelte
// components it lists), so a view removed from the registry, or missing from the rendered
// navigation, fails the test instead of silently shrinking it.

const registry = readFileSync(fileURLToPath(new URL('../../src/views/registry.ts', import.meta.url)), 'utf8')
const defs = [...registry.matchAll(/\{\s*id:\s*'([a-z-]+)',\s*label:\s*'[\w.]+',\s*place:\s*'(main|panel)'/g)].map((m) => ({ id: m[1], place: m[2] }))
const main = defs.filter((d) => d.place === 'main').map((d) => d.id)
const panel = defs.filter((d) => d.place === 'panel').map((d) => d.id)

/** The shell's regions, by test id, and the top bar by its element. */
const REGIONS = ['header.top', '[data-testid=track-list]', '[data-testid=view]', '[data-testid=bottom-panel]']

/** A region overflows when its content is larger than its box, or the box leaves the viewport. */
async function overflows(page: Page, { viewMayScroll = false } = {}): Promise<string[]> {
  const found = await page.evaluate((regions) => {
    const bad: string[] = []
    for (const selector of regions) {
      const el = document.querySelector(selector)
      if (!el) {
        bad.push(`${selector}: missing`)
        continue
      }
      const box = el.getBoundingClientRect()
      if (el.scrollWidth > el.clientWidth + 1) bad.push(`${selector}: scrollWidth ${el.scrollWidth} > ${el.clientWidth}`)
      if (el.scrollHeight > el.clientHeight + 1) bad.push(`${selector}: scrollHeight ${el.scrollHeight} > ${el.clientHeight}`)
      if (box.left < -1 || box.right > innerWidth + 1 || box.bottom > innerHeight + 1) bad.push(`${selector}: outside the viewport`)
    }
    const page = document.documentElement
    if (page.scrollWidth > innerWidth || page.scrollHeight > innerHeight) bad.push('page: scrolls')
    return bad
  }, REGIONS)
  // The piano roll takes more than half of the height (it needs rows to draw in): the main view, which scrolls on its own, may then be shorter than its content.
  return viewMayScroll ? found.filter((line) => !line.startsWith('[data-testid=view]: scrollHeight')) : found
}

// The views the shell must offer. Adding a view needs no edit here; removing one from the registry
// is a product decision that must edit this list too, instead of passing unnoticed.
const REQUIRED = ['arrangement', 'devices', 'grid', 'keyboard', 'mixer', 'piano-roll']

test('the test sees the registry', () => {
  expect(defs.map((d) => d.id)).toEqual(expect.arrayContaining(REQUIRED))
  expect(main.length).toBeGreaterThan(0)
  expect(panel.length).toBeGreaterThan(0)
})

for (const lang of ['en', 'fr'] as const) {
  for (const width of [1280, 1920]) {
    test(`navigation, ${lang}, ${width} px: every view, every panel tab, no overflow`, async ({ page }) => {
      const { errors } = watch(page)
      await page.setViewportSize({ width, height: 800 })
      await page.addInitScript((l) => localStorage.setItem('faulk.lang', l), lang)
      await page.goto('/app')
      await expect(page.locator('html')).toHaveAttribute('lang', lang)

      // The rendered navigation is exactly the registry's.
      const nav = page.locator('[data-testid^=nav-]')
      await expect(nav).toHaveCount(main.length)
      expect(await nav.evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.slice(4)))).toEqual(main)
      const tabs = page.locator('[data-testid^=panel-]')
      await expect(tabs).toHaveCount(panel.length)
      expect(await tabs.evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.slice(6)))).toEqual(panel)

      // The screen before the audio starts is the first one a user sees.
      expect(await overflows(page), 'before the audio starts').toEqual([])

      // Audio: the top bar's button, then the engine state shows the mode.
      await page.getByTestId('engine-state').getByRole('button').click()
      await engineUp(page)
      await expect(page.getByTestId('voices')).toBeVisible()

      for (const id of main) {
        const button = page.getByTestId(`nav-${id}`)
        await expect(button).not.toBeEmpty()
        await button.click()
        await expect(page.getByTestId('view')).toHaveAttribute('data-view', id)
        await expect(button).toHaveAttribute('aria-current', 'page')
        await expect(page.locator('[data-testid^=nav-][aria-current=page]')).toHaveCount(1)
        await expect(page.getByTestId('view')).toHaveAccessibleName(await button.innerText())
        expect(await overflows(page), `view ${id}`).toEqual([])
      }

      for (const id of panel) {
        const tab = page.getByTestId(`panel-${id}`)
        await tab.click()
        await expect(page.getByTestId('bottom-panel')).toHaveAttribute('data-panel', id)
        await expect(tab).toHaveAttribute('aria-pressed', 'true')
        await expect(page.locator('[data-testid^=panel-][aria-pressed=true]')).toHaveCount(1)
        // Each panel with each main view: the panel never pushes the main area out.
        for (const view of main) {
          await page.getByTestId(`nav-${view}`).click()
          await expect(page.getByTestId('view')).toHaveAttribute('data-view', view)
          expect(await overflows(page, { viewMayScroll: id === 'piano-roll' }), `panel ${id} over view ${view}`).toEqual([])
        }
      }
      expect(errors).toEqual([])
    })
  }
}

test('a note held on the computer keyboard goes on when the view changes, and ends with its key (#361)', async ({ page }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button').click()
  await expect(page.getByTestId('voices')).toHaveText('Voices: 0')
  await page.getByTestId('nav-grid').click()
  await page.keyboard.down('KeyA')
  await expect(page.getByTestId('voices')).toHaveText('Voices: 1')
  // The keyboard plays in every view: changing view with the key still down keeps the note, the key ends it.
  const other = main.find((id) => id !== 'grid')!
  await page.getByTestId(`nav-${other}`).click()
  await expect(page.getByTestId('view')).toHaveAttribute('data-view', other)
  await expect(page.getByTestId('voices')).toHaveText('Voices: 1')
  await page.keyboard.up('KeyA')
  await expect(page.getByTestId('voices')).toHaveText('Voices: 0', { timeout: 5000 })
  expect(errors).toEqual([])
})
