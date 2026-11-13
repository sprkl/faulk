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

/**
 * The whole surface of every link of a bar (`header` or `footer`), hit-tested inside the page: one `evaluate` per bar
 * (#848), where one round trip per link and position took 30 to 50 s a test. Every point of a 7 x 5 grid over the link's
 * box, three pixels in (the language switch's rounded border and padding are the list's, not the link's), the four corner
 * points skipped (the corners of a pill-shaped link are not its surface), must be the link or inside it.
 * The footer is sampled first while it scrolls in (its reveal follows the scroll, `animation-timeline: view()`): at 1000,
 * 700, 400, 200 and 0 px from the bottom, every link then wholly on screen, so the wordmark is hit-tested while it is still
 * moving into place. Then, once the page's timed animations have ended, every link of the bar is scrolled into view if
 * needed and hit-tested once its box holds still (what `scrollIntoViewIfNeeded` waited for), and again after a scroll to
 * the bottom for the footer. Each scroll waits two frames, for the scroll-driven styles. Gives the number of links and
 * the points another element caught.
 */
function surfaceOfTheLinks(page: Page, bar: 'header' | 'footer') {
  return page.evaluate(async (which) => {
    const frames = () => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done())))
    const same = (a: DOMRect, b: DOMRect) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height
    // A box that holds still over two frames, tried for about 10 s (Playwright's wait failed on a link that never settles).
    async function still(a: Element) {
      for (let i = 0; i < 300; i++) {
        const before = a.getBoundingClientRect()
        await frames()
        if (same(before, a.getBoundingClientRect())) return true
      }
      return false
    }
    const links = [...document.querySelectorAll(`${which} a`)]
    const stolen: string[] = []
    const hitTest = (a: Element, n: number, when: string) => {
      const box = a.getBoundingClientRect()
      for (let i = 0; i <= 6; i++) {
        for (let j = 0; j <= 4; j++) {
          if ((i === 0 || i === 6) && (j === 0 || j === 4)) continue
          const x = box.left + 3 + ((box.width - 6) * i) / 6
          const y = box.top + 3 + ((box.height - 6) * j) / 4
          const hit = document.elementFromPoint(x, y)
          if (hit !== a && !a.contains(hit))
            stolen.push(`${when}, link ${n} (${a.getAttribute('href')}): ${Math.round(x)},${Math.round(y)} -> ${hit?.className || hit?.tagName}`)
        }
      }
    }
    if (which === 'footer') {
      for (const fromBottom of [1000, 700, 400, 200, 0]) {
        window.scrollTo(0, Math.max(0, document.body.scrollHeight - innerHeight - fromBottom))
        await frames()
        links.forEach((a, n) => {
          const box = a.getBoundingClientRect()
          if (box.top >= 0 && box.bottom <= innerHeight) hitTest(a, n, `scrolling in, ${fromBottom} px from the bottom`)
        })
      }
      // The timed animations (the hero's entrance) end; the endless ones (the grain) and the scroll-driven ones never do.
      const timed = document.getAnimations().filter((a) => a.timeline === document.timeline && Number.isFinite(Number(a.effect?.getComputedTiming().endTime)))
      await Promise.all(timed.map((a) => a.finished.catch(() => undefined)))
    }
    for (const pass of ['revealed', 'at the bottom']) {
      if (pass === 'at the bottom' && which === 'footer') window.scrollTo(0, document.body.scrollHeight)
      for (const [n, a] of links.entries()) {
        ;(a as Element & { scrollIntoViewIfNeeded?: () => void }).scrollIntoViewIfNeeded?.()
        if (await still(a)) hitTest(a, n, pass)
        else stolen.push(`${pass}, link ${n} (${a.getAttribute('href')}): its box never holds still`)
      }
    }
    return { count: links.length, stolen }
  }, bar)
}

for (const lang of LANGS) {
  for (const [width, height] of WIDTHS) {
    test(`landing, ${lang}, ${width} px: the whole surface of every footer and top-bar link is the link's, while scrolling in and once revealed`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height })
      await page.goto(`/${lang}`)
      for (const bar of ['header', 'footer'] as const) {
        const { count, stolen } = await surfaceOfTheLinks(page, bar)
        expect(count, `links of the ${bar}`).toBeGreaterThanOrEqual(7)
        expect(stolen, `points of the ${bar} links that another element catches`).toEqual([])
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

        // Open Faulk: the click reaches the DAW. Its boot is not waited for (#848): every DAW spec boots it and watches its
        // errors, and its full load here was one more DAW start per test.
        await open()
        await clickToReach(page, links(page, bar).and(page.locator('a[href$="/app"]')), /\/app\/?$/)

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
