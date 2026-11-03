import { expect, test, type Page } from '@playwright/test'
import { watch } from './support'

// The Journey page (#495): prerendered, readable without JavaScript, with its own players and a
// visualiser. The audio comes from a CDN in production; here every `track.mp3` is answered with a
// short generated WAV (open CORS, like the CDN), so the test never depends on the network.

/** A 4 s stereo 16-bit WAV: a 220 Hz tone, loud enough for the analyser to see. */
function tone(seconds = 4, rate = 22050): Buffer {
  const frames = seconds * rate
  const data = Buffer.alloc(frames * 4)
  for (let i = 0; i < frames; i++) {
    const v = Math.round(Math.sin((2 * Math.PI * 220 * i) / rate) * 0.5 * 32767)
    data.writeInt16LE(v, i * 4)
    data.writeInt16LE(v, i * 4 + 2)
  }
  const head = Buffer.alloc(44)
  head.write('RIFF', 0)
  head.writeUInt32LE(36 + data.length, 4)
  head.write('WAVEfmt ', 8)
  head.writeUInt32LE(16, 16)
  head.writeUInt16LE(1, 20)
  head.writeUInt16LE(2, 22)
  head.writeUInt32LE(rate, 24)
  head.writeUInt32LE(rate * 4, 28)
  head.writeUInt16LE(4, 32)
  head.writeUInt16LE(16, 34)
  head.write('data', 36)
  head.writeUInt32LE(data.length, 40)
  return Buffer.concat([head, data])
}

const WAV = tone()

async function fakeCdn(page: Page) {
  await page.route('https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/**/track.mp3', (route) =>
    route.fulfill({ status: 200, body: WAV, headers: { 'content-type': 'audio/wav', 'access-control-allow-origin': '*' } }),
  )
}

const players = (page: Page) => page.locator('[data-fk-player]')

for (const lang of ['en', 'fr'] as const) {
  test(`journey, ${lang}: every entry shows its cover, player, texts and links, with no console error`, async ({ page }) => {
    const { errors } = watch(page)
    await fakeCdn(page)
    await page.goto(`/${lang}/journey`)
    await expect(page.locator('html')).toHaveAttribute('lang', lang)
    await expect(page.locator('h1')).toBeVisible()
    const articles = page.locator('main article')
    const count = await articles.count()
    expect(count).toBeGreaterThanOrEqual(3)
    // The index lists the latest days (up to 4); the months follow the feed.
    await expect(page.locator('main nav.index ol > li')).toHaveCount(Math.min(4, count))
    await expect(page.locator('#archive a').first()).toHaveAttribute('href', new RegExp(`${lang}/journey/\\d{4}-\\d{2}$`))
    // Every entry's title leads to its own page.
    await expect(articles.locator('a.permalink')).toHaveCount(count)
    for (let i = 0; i < count; i++) {
      const a = articles.nth(i)
      await expect(a.locator('svg[role=img]')).toHaveAttribute('aria-label', /.+/)
      await expect(a.locator('[data-fk-player]')).toHaveCount(1)
      await expect(a.locator('a[href*="faulk-journey/blob/main/tracks/"][href$="/sheet.md"]')).toHaveCount(1)
      await expect(a.locator('a[href$="/track.mid"]')).toHaveCount(1)
      await expect(a.locator('a[href^="https://github.com/sprkl/faulk/issues/362#issuecomment-"]')).toHaveCount(1)
      // The day's project opens in Faulk (#497): the DAW, asked to open the pack on jsDelivr.
      const open = a.getByTestId('open-in-faulk')
      await expect(open).toHaveText(lang === 'en' ? 'Open in Faulk' : 'Ouvrir dans Faulk')
      const href = new URL((await open.getAttribute('href'))!, page.url())
      expect(href.pathname).toBe('/app')
      expect(href.searchParams.get('open')).toMatch(
        /^https:\/\/cdn\.jsdelivr\.net\/gh\/sprkl\/faulk-journey@main\/tracks\/\d{4}-\d{2}-\d{2}-[a-z0-9-]+\/project\.faulkpack$/,
      )
      await expect(a.locator('h2, h3').first()).not.toBeEmpty()
      // The style profile a run applied is said once, discreetly, by the entry that has one (#506).
      const style = a.getByTestId('entry-style')
      if ((await a.getAttribute('id')) === 'day-2026-11-03')
        await expect(style).toHaveText(lang === 'en' ? 'Style profile: morzak' : 'Profil de style : morzak')
      else if ((await a.getAttribute('id')) === 'day-2026-10-31') await expect(style).toHaveCount(0)
    }
    // The page's own controls replaced the native ones.
    await expect(page.locator('html')).toHaveAttribute('data-fk-players', String(count))
    await expect(page.locator('[data-fk-controls]').first()).toBeVisible()
    await expect(page.locator('audio[data-fk-audio]').first()).toBeHidden()
    // The licence line.
    await expect(page.locator('a[rel~=license][href="https://creativecommons.org/licenses/by/4.0/"]')).toHaveCount(1)
    // The landing page's navigation leads here, and this page's language switch stays on it.
    await expect(page.locator('header nav a[aria-current=page]')).toHaveAttribute('href', new RegExp(`(^|/)${lang}/journey$`))
    await expect(page.locator('a[data-lang=fr]').first()).toHaveAttribute('href', new RegExp('(^|/)fr/journey$'))
    expect(errors).toEqual([])
  })
}

for (const lang of ['en', 'fr'] as const) {
  test(`journey, ${lang}: the day's own page says the style profile too (#506)`, async ({ page }) => {
    const { errors } = watch(page)
    await fakeCdn(page)
    await page.goto(`/${lang}/journey/2026-11-03-brume-sur-le-canal`)
    await expect(page.getByTestId('entry-style')).toHaveText(lang === 'en' ? 'Style profile: morzak' : 'Profil de style : morzak')
    await page.goto(`/${lang}/journey/2026-10-31-ligne-haute`)
    await expect(page.getByTestId('entry-style')).toHaveCount(0)
    expect(errors).toEqual([])
  })
}

test('journey: /journey moves a French reader to /fr/journey, and the landing page links to it', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
  await page.goto('/journey')
  await expect(page).toHaveURL(/\/fr\/journey$/)
  await page.goto('/en')
  await expect(page.locator('header nav a[href$="en/journey"]')).toHaveText('Journey')
})

test('journey: without JavaScript the page reads, with the native audio controls', async ({ browser, baseURL }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, baseURL })
  const page = await context.newPage()
  await page.goto('/en/journey')
  await expect(page.locator('h1')).toBeVisible()
  await expect(page.locator('main article').first()).toContainText(/\S/)
  const audio = page.locator('audio[data-fk-audio]').first()
  await expect(audio).toBeVisible()
  await expect(audio).toHaveAttribute('controls', '')
  await expect(audio).toHaveAttribute('crossorigin', 'anonymous')
  await expect(page.locator('[data-fk-controls]').first()).toBeHidden()
  await context.close()
})

test('journey: play, pause, seek from the keyboard, one track at a time, and the visualiser runs', async ({ page }) => {
  const { errors } = watch(page)
  await fakeCdn(page)
  await page.goto('/en/journey')
  const first = players(page).nth(0)
  const second = players(page).nth(1)
  const play = first.locator('button[data-fk-play]')
  await expect(play).toHaveAttribute('aria-label', /^Play /)

  // Keyboard: Tab to the button, Space plays.
  await play.focus()
  await page.keyboard.press('Space')
  await expect(first).toHaveAttribute('data-state', 'playing')
  await expect(play).toHaveAttribute('aria-label', /^Pause /)
  await expect.poll(() => first.locator('audio').evaluate((a: HTMLAudioElement) => a.currentTime)).toBeGreaterThan(0.3)
  // The media's own length replaced the stated one.
  await expect(first.locator('[data-fk-total]')).toHaveText('0:04')

  // The visualiser draws over the cover, fed by the analyser.
  const stage = page.locator(`#${await first.getAttribute('data-fk-stage-id')}`)
  await expect(stage).toHaveClass(/is-live/)
  await expect(stage.locator('canvas.fk-live-canvas')).toHaveCount(1)
  await expect
    .poll(() =>
      stage.locator('canvas').evaluate((c: HTMLCanvasElement) => {
        // Some pixel off the ink background: the foci and the waveform are drawn.
        const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data
        let lit = 0
        for (let i = 0; i < d.length; i += 4 * 97) if (d[i] + d[i + 1] + d[i + 2] > 200) lit++
        return lit
      }),
    )
    .toBeGreaterThan(0)

  // Seek with the keyboard: End, then Home; the spoken value follows.
  const seek = first.locator('input[data-fk-seek]')
  await seek.focus()
  await page.keyboard.press('Home')
  await expect(seek).toHaveAttribute('aria-valuetext', /^0:0\d of 0:04$/)
  await page.keyboard.press('ArrowRight')
  await expect.poll(() => first.locator('audio').evaluate((a: HTMLAudioElement) => a.currentTime)).toBeGreaterThanOrEqual(3)

  // Another track starts: the first one stops.
  await second.locator('button[data-fk-play]').click()
  await expect(second).toHaveAttribute('data-state', 'playing')
  await expect(first).not.toHaveAttribute('data-state', 'playing')
  expect(await first.locator('audio').evaluate((a: HTMLAudioElement) => a.paused)).toBe(true)

  // Pause.
  await second.locator('button[data-fk-play]').click()
  await expect(second).toHaveAttribute('data-state', 'paused')
  await expect(page.locator('.is-live')).toHaveCount(0)
  expect(errors).toEqual([])
})

test('journey: under reduced motion the track plays with no visualiser', async ({ browser, baseURL }) => {
  const context = await browser.newContext({ reducedMotion: 'reduce', baseURL })
  const page = await context.newPage()
  const { errors } = watch(page)
  await fakeCdn(page)
  await page.goto('/en/journey')
  const first = players(page).nth(0)
  await first.locator('button[data-fk-play]').click()
  await expect(first).toHaveAttribute('data-state', 'playing')
  await expect.poll(() => first.locator('audio').evaluate((a: HTMLAudioElement) => a.currentTime)).toBeGreaterThan(0.3)
  await page.waitForTimeout(500)
  await expect(page.locator('canvas.fk-live-canvas')).toHaveCount(0)
  expect(errors).toEqual([])
  await context.close()
})

test('journey: reduced motion switched on during playback drops the visualiser, the track plays on, and it comes back once switched off', async ({ page }) => {
  const { errors } = watch(page)
  await fakeCdn(page)
  await page.goto('/en/journey')
  const first = players(page).nth(0)
  const play = first.locator('button[data-fk-play]')
  await play.click()
  await expect(page.locator('canvas.fk-live-canvas')).toHaveCount(1)
  await page.emulateMedia({ reducedMotion: 'reduce' })
  await expect(page.locator('canvas.fk-live-canvas')).toHaveCount(0)
  await expect(first).toHaveAttribute('data-state', 'playing')
  const t = await first.locator('audio').evaluate((a: HTMLAudioElement) => a.currentTime)
  await expect.poll(() => first.locator('audio').evaluate((a: HTMLAudioElement) => a.currentTime)).toBeGreaterThan(t)
  // Back to motion: the next play brings the visualiser back.
  await page.emulateMedia({ reducedMotion: 'no-preference' })
  await play.click()
  await expect(first).toHaveAttribute('data-state', 'paused')
  await play.click()
  await expect(page.locator('canvas.fk-live-canvas')).toHaveCount(1)
  expect(errors).toEqual([])
})

test('journey: an audio file that fails says so and offers the link', async ({ page }) => {
  await page.route('https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/**/track.mp3', (route) => route.fulfill({ status: 404, body: '' }))
  await page.goto('/en/journey')
  const first = players(page).nth(0)
  await first.locator('button[data-fk-play]').click()
  await expect(first).toHaveAttribute('data-state', 'error')
  await expect(first.locator('[data-fk-error]')).toBeVisible()
  await expect(first.locator('button[data-fk-play]')).toBeDisabled()
})

test('journey: no audio is fetched before a play', async ({ page }) => {
  const audio: string[] = []
  page.on('request', (r) => {
    if (/\/track\.mp3$/.test(r.url())) audio.push(r.url())
  })
  await fakeCdn(page)
  await page.goto('/en/journey')
  await expect(page.locator('html')).toHaveAttribute('data-fk-players', /\d+/)
  await page.mouse.wheel(0, 4000)
  await page.waitForTimeout(500)
  expect(audio).toEqual([])
  for (const el of await page.locator('audio[data-fk-audio]').all()) await expect(el).toHaveAttribute('preload', 'none')
  await players(page).first().locator('button[data-fk-play]').click()
  await expect.poll(() => audio.length).toBeGreaterThan(0)
  expect(new Set(audio).size).toBe(1)
})

for (const lang of ['en', 'fr'] as const) {
  test(`journey, ${lang}: a month's archive and a day's page, with the way between them`, async ({ page }) => {
    const { errors } = watch(page)
    await fakeCdn(page)
    await page.goto(`/${lang}/journey`)
    // The months list leads to an archive of that month's days.
    await page.locator('#archive a').first().click()
    await expect(page).toHaveURL(new RegExp(`/${lang}/journey/\\d{4}-\\d{2}$`))
    await expect(page.locator('h1')).not.toBeEmpty()
    const days = page.locator('main article')
    expect(await days.count()).toBeGreaterThan(0)
    // Every day of the archive can be opened in Faulk too.
    await expect(days.getByTestId('open-in-faulk')).toHaveCount(await days.count())
    await expect(page.locator('#archive a[aria-current=page]')).toHaveCount(1)
    // A day's title leads to its own page: the entry with its player and its actions, as everywhere.
    const title = (await days.first().locator('a.permalink').textContent())!.trim()
    await days.first().locator('a.permalink').click()
    await expect(page).toHaveURL(new RegExp(`/${lang}/journey/\\d{4}-\\d{2}-\\d{2}-[a-z0-9-]+$`))
    await expect(page.locator('h1')).toHaveText(title)
    await expect(page).toHaveTitle(new RegExp(title))
    await expect(page.locator('[data-fk-player]')).toHaveCount(1)
    await expect(page.locator('[data-fk-actions] a')).toHaveCount(4)
    // The language switch stays on this day.
    const other = lang === 'en' ? 'fr' : 'en'
    const day = new URL(page.url()).pathname.split('/').pop()!
    await expect(page.locator(`header a[data-lang=${other}]`)).toHaveAttribute('href', new RegExp(`${other}/journey/${day}$`))
    // Back to its month.
    await page.locator('.crumbs a').nth(1).click()
    await expect(page).toHaveURL(new RegExp(`/${lang}/journey/\\d{4}-\\d{2}$`))
    expect(errors).toEqual([])
  })
}

test('journey: an unprefixed day page moves a French reader to /fr', async ({ page }) => {
  await page.goto('/en/journey')
  const day = (await page.locator('a.permalink').first().getAttribute('href'))!.split('/').pop()!
  await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
  await page.goto(`/journey/${day}`)
  await expect(page).toHaveURL(new RegExp(`/fr/journey/${day}$`))
})
