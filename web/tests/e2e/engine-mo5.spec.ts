import { expect, test, type Page } from '@playwright/test'
import { fakeWebMidi, watch } from './support'

// MO-5 (#58): the engine's report in the real page. The hidden `feedback` element carries the
// decoded report as data attributes (the mixer and the transport views, MX-3 and TR-4, will draw
// it). Web mode: the render instance writes it into the shared memory, the page reads it; no
// message crosses the audio thread.

test.describe.configure({ mode: 'serial' })

const feedback = (page: Page) => page.getByTestId('feedback')
const number = async (page: Page, attribute: string) => Number(await feedback(page).getAttribute(attribute))
const send = (page: Page, bytes: number[]) => page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)

async function start(page: Page) {
  await fakeWebMidi(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  await page.getByRole('radio', { name: 'E2E Keys' }).check()
}

test('the meters move with a note, per track and on the output, and fall back at silence', async ({ page }) => {
  const { errors } = watch(page)
  await start(page)
  await expect.poll(() => number(page, 'data-seq')).toBeGreaterThan(0)
  expect(await number(page, 'data-master-peak')).toBe(0)
  await send(page, [0x90, 69, 127])
  await expect.poll(() => number(page, 'data-master-peak')).toBeGreaterThan(0.05)
  const tracks = (await feedback(page).getAttribute('data-track-peaks'))!.split(',').map(Number)
  expect(tracks.length).toBeGreaterThanOrEqual(1)
  expect(tracks[0]).toBeGreaterThan(0.05)
  await send(page, [0x80, 69, 0])
  await expect.poll(() => number(page, 'data-master-peak'), { timeout: 5000 }).toBeLessThan(0.001)
  expect(errors).toEqual([])
})

test('the playhead advances while the transport plays and stops with it', async ({ page }, info) => {
  test.skip(info.project.name !== 'dev', 'the command hook exists on the dev server only')
  const { errors } = watch(page)
  await start(page)
  const run = (command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): void }).__faulkRun(c), command)
  await expect.poll(() => number(page, 'data-seq')).toBeGreaterThan(0)
  expect(await feedback(page).getAttribute('data-transport')).toBe('stopped')
  expect(await number(page, 'data-tick')).toBe(0)
  await run({ type: 'transport.play' })
  await expect(feedback(page)).toHaveAttribute('data-transport', 'playing')
  const first = await expect
    .poll(() => number(page, 'data-tick'))
    .toBeGreaterThan(0)
    .then(() => number(page, 'data-tick'))
  await expect.poll(() => number(page, 'data-tick'), { timeout: 5000 }).toBeGreaterThan(first + 100)
  await run({ type: 'transport.stop' })
  await expect(feedback(page)).toHaveAttribute('data-transport', 'stopped')
  const stopped = await number(page, 'data-tick')
  await page.waitForTimeout(300)
  expect(await number(page, 'data-tick')).toBe(stopped)
  expect(errors).toEqual([])
})
