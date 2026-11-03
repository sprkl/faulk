import { createServer, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { readFileSync } from 'node:fs'
import { expect, test, type Browser, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #497: a `.faulkpack` hosted somewhere opens by its link, `/app?open=<link>`. The page asks first and
// names the host, the parameter leaves the address, the pack opens as a new project in the browser's
// storage, and the project that was open stays in the list. The pack is made in the page (Export as
// .faulkpack) and served by a small HTTP server on this computer; an `https` link is answered by the
// test's routing. Failures: a host that does not answer, a file that is no pack, a link the page refuses.

test.describe.configure({ timeout: 90_000 })
test.use({ storageState: { cookies: [], origins: [] } })

const tracks = (page: Page) => page.getByTestId('track-header')

async function start(page: Page) {
  await page.getByTestId('engine-state').getByRole('button').click()
  await engineUp(page)
}

async function addTrack(page: Page, item: string) {
  await page.getByTestId('add-track').click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}

let server: Server
let origin: string
let pack: Buffer

/** Makes a pack of three tracks in a page of its own (Export as .faulkpack). */
async function makePack(browser: Browser, path: string): Promise<Buffer> {
  const context = await browser.newContext({ locale: 'en-US', viewport: { width: 1440, height: 900 } })
  const page = await context.newPage()
  await page.goto('/app')
  await start(page)
  await addTrack(page, 'Drums')
  await addTrack(page, 'Drums')
  await expect(tracks(page)).toHaveCount(3)
  await page.getByTestId('project-menu').click()
  await expect(page.getByTestId('menu-export-pack')).toHaveAttribute('title', /\?open=/)
  const [download] = await Promise.all([page.waitForEvent('download'), page.getByTestId('menu-export-pack').click()])
  await download.saveAs(path)
  await context.close()
  return readFileSync(path)
}

test.beforeAll(async ({ browser }, testInfo) => {
  pack = await makePack(browser, testInfo.outputPath('song.faulkpack'))
  server = createServer((request, response) => {
    // Open CORS and a cross-origin resource policy, as a CDN sends them.
    const headers = { 'Access-Control-Allow-Origin': '*', 'Cross-Origin-Resource-Policy': 'cross-origin', 'Content-Type': 'application/octet-stream' }
    if (request.url === '/song.faulkpack') response.writeHead(200, headers).end(pack)
    else if (request.url === '/bad.faulkpack') response.writeHead(200, headers).end('this is not a zip file')
    else response.writeHead(404, headers).end()
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`
})

test.afterAll(async () => {
  await new Promise((resolve) => server?.close(resolve))
})

const openLink = (link: string) => `/app?open=${encodeURIComponent(link)}`

test('asks first, naming the host; Cancel downloads nothing and the link leaves the address', async ({ page }) => {
  const { errors, requests } = watch(page)
  await page.goto(openLink(`${origin}/song.faulkpack`))
  const dialog = page.getByTestId('open-link')
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('heading')).toHaveText(`Open a project from ${new URL(origin).host}?`)
  await expect(page.getByTestId('open-link-url')).toHaveText(`${origin}/song.faulkpack`)
  await expect(page.getByTestId('open-link-cancel')).toBeFocused()
  await expect(page).toHaveURL(/\/app$/)
  await page.keyboard.press('Escape')
  await expect(dialog).toHaveCount(0)
  expect(requests.filter((url) => url.startsWith(origin))).toEqual([])
  // A reload does not ask again.
  await page.reload()
  await expect(page.getByTestId('engine-state')).toBeVisible()
  await expect(dialog).toHaveCount(0)
  expect(errors).toEqual([])
})

test('opens the pack as a new project and keeps the one that was open in the list', async ({ page }) => {
  const { errors } = watch(page)
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await start(page)
  await addTrack(page, 'Drums')
  await expect(tracks(page)).toHaveCount(2)
  await expect(page.getByTestId('save-state')).toHaveText(/^Draft saved/, { timeout: 20_000 })

  await page.goto(openLink(`${origin}/song.faulkpack`))
  await page.getByTestId('open-link-open').click()
  await expect(tracks(page)).toHaveCount(3)
  await expect(page).toHaveURL(/\/app$/)
  await expect(page.getByTestId('download-banner')).toHaveCount(0)
  // The project of before is a draft in the list, and comes back from it whole.
  await page.getByTestId('project-menu').click()
  await expect(page.getByTestId('menu-browser-project')).toHaveCount(2)
  await page.keyboard.press('Escape')
  // Both are named alike: opening each in turn, one of them is the project of before, with its two tracks.
  const counts: number[] = []
  for (const at of [0, 1]) {
    await page.getByTestId('project-menu').click()
    await page.getByTestId('menu-browser-project').nth(at).click()
    await page.waitForTimeout(500)
    await expect(page.getByTestId('confirm')).toHaveCount(0)
    counts.push(await tracks(page).count())
  }
  expect(counts.sort()).toEqual([2, 3])
  expect(errors).toEqual([])
})

test('the same link again opens the copy the browser keeps, not a second one', async ({ page }) => {
  await page.goto(openLink(`${origin}/song.faulkpack`))
  await page.getByTestId('open-link-open').click()
  await expect(tracks(page)).toHaveCount(3)
  await page.goto(openLink(`${origin}/song.faulkpack`))
  await page.getByTestId('open-link-open').click()
  await expect(page.getByTestId('storage-notice')).toContainText(/already (open|in this browser)/)
  await page.getByTestId('project-menu').click()
  await expect(page.getByTestId('menu-browser-project')).toHaveCount(1)
})

test('a host that does not answer is said with its name, and the file import is offered', async ({ page }) => {
  await page.goto(openLink(`${origin}/missing.faulkpack`))
  await page.getByTestId('open-link-open').click()
  const failed = page.getByTestId('download-failed')
  await expect(failed).toContainText(`Couldn’t download this project from ${new URL(origin).host}.`)
  // Nothing was opened: the page starts on its own default project.
  await start(page)
  await expect(tracks(page)).toHaveCount(1)
  const [chooser] = await Promise.all([page.waitForEvent('filechooser'), page.getByTestId('download-import').click()])
  expect(chooser.isMultiple()).toBe(false)
  await expect(failed).toHaveCount(0)
})

test('a file that is no pack changes nothing, and says so', async ({ page }) => {
  await page.goto(openLink(`${origin}/bad.faulkpack`))
  await page.getByTestId('open-link-open').click()
  await expect(page.getByTestId('storage-notice')).toContainText('not a valid .faulkpack')
  await start(page)
  await expect(tracks(page)).toHaveCount(1)
})

test('a link the page does not fetch is refused at once, with no question', async ({ page }) => {
  const { requests } = watch(page)
  await page.goto(openLink('http://example.org/song.faulkpack'))
  await expect(page.getByTestId('storage-notice')).toContainText('https links only')
  await expect(page.getByTestId('open-link')).toHaveCount(0)
  await expect(page).toHaveURL(/\/app$/)
  expect(requests.filter((url) => new URL(url).hostname === 'example.org')).toEqual([])
})

test('an https link opens too, without cookies; in French', async ({ browser }) => {
  const context = await browser.newContext({ locale: 'fr-FR', viewport: { width: 1440, height: 900 } })
  const page = await context.newPage()
  const link = 'https://packs.example.test/tracks/song.faulkpack'
  let credentials: string | null = 'unset'
  await page.route(link, async (route) => {
    credentials = (await route.request().allHeaders())['cookie'] ?? null
    await route.fulfill({ status: 200, body: pack, headers: { 'Access-Control-Allow-Origin': '*', 'Cross-Origin-Resource-Policy': 'cross-origin' } })
  })
  await page.goto(openLink(link))
  await expect(page.getByTestId('open-link').getByRole('heading')).toHaveText('Ouvrir un projet depuis packs.example.test ?')
  await expect(page.getByTestId('open-link')).toContainText('Ton projet en cours est gardé')
  await page.getByTestId('open-link-open').click()
  await expect(tracks(page)).toHaveCount(3)
  expect(credentials).toBeNull()
  await context.close()
})
