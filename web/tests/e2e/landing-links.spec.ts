import { expect, test, type Locator, type Page } from '@playwright/test'
import { watch } from './support'

// #587: the footer links of the landing page did nothing on click, because the giant wordmark under them caught the
// clicks. Every link of the top bar and of the footer, on the landing page and on the Journey page, in both languages,
// is hit-tested over its whole surface and clicked for real; each click must reach its destination.

const LANGS = ['en', 'fr'] as const
const OTHER = { en: 'fr', fr: 'en' } as const
const WIDTHS = [
  [390, 844],
  [1280, 720],
  [1440, 900],
  [1920, 1080],
] as const

const SECTIONS = ['modes', 'boost', 'modules'] as const

/** The links of the bar (`header`) or of the footer (`footer`). */
const links = (page: Page, bar: 'header' | 'footer') => page.locator(`${bar} a`)

/**
 * Clicks a link and waits for the page to be at `url`. The wait ends when the navigation commits, not on the `load` event:
 * the destination's own load (a dev server compiling modules on a loaded machine) is not what is checked here, and a stalled
 * one held the whole test for its 90 s (#611). A click lost to a layout move between the mouse going down and up leaves the
 * page where it was: it is made once more after 15 s, and a second failure names the page it stayed on.
 */
async function clickToReach(page: Page, link: Locator, url: RegExp) {
  for (let attempt = 1; ; attempt++) {
    await link.click({ timeout: 15_000 })
    try {
      await page.waitForURL(url, { waitUntil: 'commit', timeout: 15_000 })
      return
    } catch (error) {
      if (attempt === 2) throw new Error(`the click did not reach ${url}: still on ${page.url()}`, { cause: error })
    }
  }
}

/** Every point of a grid over the link's box (corners and edges included, a pixel inside) must belong to the link. */
async function wholeSurfaceIsTheLinks(link: Locator, scroll = true) {
  if (scroll) await link.scrollIntoViewIfNeeded()
  const stolen = await link.evaluate((a) => {
    // Three pixels in: the language switch's rounded border and padding are the list's, not the link's.
    const box = a.getBoundingClientRect()
    const out: string[] = []
    for (let i = 0; i <= 6; i++) {
      for (let j = 0; j <= 4; j++) {
        // The corners of a pill-shaped link are not its surface.
        if ((i === 0 || i === 6) && (j === 0 || j === 4)) continue
        const x = box.left + 3 + ((box.width - 6) * i) / 6
        const y = box.top + 3 + ((box.height - 6) * j) / 4
        const hit = document.elementFromPoint(x, y)
        if (hit !== a && !a.contains(hit)) out.push(`${Math.round(x)},${Math.round(y)} -> ${hit?.className || hit?.tagName}`)
      }
    }
    return out
  })
  expect(stolen, 'points of the link that another element catches').toEqual([])
}

for (const lang of LANGS) {
  for (const [width, height] of WIDTHS) {
    test(`landing, ${lang}, ${width} px: the whole surface of every footer and top-bar link is the link's, while scrolling in and once revealed`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height })
      await page.goto(`/${lang}`)
      for (const bar of ['header', 'footer'] as const) {
        const all = links(page, bar)
        const count = await all.count()
        expect(count).toBeGreaterThanOrEqual(7)
        if (bar === 'footer') {
          // The reveal follows the scroll (`animation-timeline: view()`): sample the links where they are on screen at
          // several scroll positions on the way down, so the wordmark is hit-tested while it is still moving into place.
          for (const fromBottom of [1000, 700, 400, 200, 0]) {
            await page.evaluate((offset) => window.scrollTo(0, Math.max(0, document.body.scrollHeight - innerHeight - offset)), fromBottom)
            for (let i = 0; i < count; i++) {
              const onScreen = await all.nth(i).evaluate((a) => {
                const box = a.getBoundingClientRect()
                return box.top >= 0 && box.bottom <= innerHeight
              })
              if (onScreen) await wholeSurfaceIsTheLinks(all.nth(i), false)
            }
          }
          await page.waitForTimeout(1000)
        }
        for (let i = 0; i < count; i++) await wholeSurfaceIsTheLinks(all.nth(i))
        if (bar === 'footer') await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight))
        for (let i = 0; i < count; i++) await wholeSurfaceIsTheLinks(all.nth(i))
      }
    })
  }
}

for (const lang of LANGS) {
  for (const from of ['landing', 'journey'] as const) {
    const path = from === 'landing' ? `/${lang}` : `/${lang}/journey`
    for (const bar of ['header', 'footer'] as const) {
      test(`${from}, ${lang}: each ${bar} link, clicked, reaches its destination`, async ({ page }) => {
        await page.setViewportSize({ width: 1440, height: 900 })
        const { errors } = watch(page)
        const open = async () => {
          await page.goto(path)
          await expect(page.locator('html')).toHaveAttribute('lang', lang)
        }

        // Sections: the landing page scrolls to it, the Journey page goes back to the landing page at that fragment.
        for (const id of SECTIONS) {
          await open()
          const a = links(page, bar).and(page.locator(`a[href$="#${id}"]`))
          await expect(a).toHaveCount(1)
          await clickToReach(page, a, new RegExp(`/${lang}/?#${id}$`))
          await expect(page.locator(`#${id}`)).toBeVisible()
          await expect.poll(() => page.locator(`#${id}`).evaluate((s) => s.getBoundingClientRect().top < window.innerHeight)).toBe(true)
        }

        // Journey.
        await open()
        await clickToReach(page, links(page, bar).and(page.locator(`a:not([data-lang])[href$="/${lang}/journey"]`)), new RegExp(`/${lang}/journey/?$`))
        await expect(page.locator('h1')).toBeVisible()

        // The language switch keeps the page: the landing page goes to the other landing page, the Journey page to its own.
        await open()
        const other = OTHER[lang]
        await clickToReach(
          page,
          links(page, bar).and(page.locator(`a[data-lang="${other}"]`)),
          new RegExp(`/${other}${from === 'journey' ? '/journey' : ''}/?$`),
        )
        await expect(page.locator('html')).toHaveAttribute('lang', other, { timeout: 30_000 })

        // Open Faulk: a full page load of the DAW.
        await open()
        await clickToReach(page, links(page, bar).and(page.locator('a[href$="/app"]')), /\/app\/?$/)
        // The DAW's boot is part of this step: its errors are read once it has loaded.
        await page.waitForLoadState('load', { timeout: 60_000 })

        expect(errors).toEqual([])
      })
    }
  }

  test(`landing, ${lang}: the brand link in the bar goes home, from the Journey page and from the landing page`, async ({ page }) => {
    await page.goto(`/${lang}/journey`)
    await page.locator('header a.brand').click()
    await page.waitForURL(new RegExp(`/${lang}/?$`))
    await page.goto(`/${lang}#boost`)
    await page.locator('header a.brand').click()
    await page.waitForURL(new RegExp(`/${lang}/?$`))
    await expect(page.locator('html')).toHaveAttribute('lang', lang)
  })

  test(`landing, ${lang}: Boost offers the Windows installer as a link and macOS as not available yet`, async ({ page }) => {
    await page.goto(`/${lang}`)
    const windows = page.getByTestId('boost-windows')
    await expect(windows).toHaveAttribute('href', 'https://get.faulk.app/helper/win/stable')
    await expect(page.locator('#boost a[href*="get.faulk.app"]')).toHaveCount(1)
    await expect(page.locator('#boost button:disabled')).toHaveCount(1)
    await expect(page.locator('#boost .next')).toContainText('Open Faulk')
    // A real click reaches the link (nothing covers it); the navigation itself is answered locally.
    let asked = ''
    await page.route('https://get.faulk.app/**', (route) => {
      asked = route.request().url()
      return route.fulfill({ status: 200, contentType: 'text/html', body: '<title>installer</title>' })
    })
    await windows.click()
    await expect.poll(() => asked).toBe('https://get.faulk.app/helper/win/stable')
  })
}
