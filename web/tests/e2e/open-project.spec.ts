import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { expect, test, type Page } from '@playwright/test'
import { fakeWebMidi, watch } from './support'

// SV-4 (#94): a project folder opened into the real page. Real worklet, engine and project core:
// the note held on the old project stops with it, the opened project's tracks and its session
// (the solo) show, and a folder that is not a project changes nothing and says so. The folder is
// the reference one of the folder format; the dispatcher's dev hook (`__faulkOpen`) stands in for
// the folder store of SV-2, so this spec runs in the dev project only.

const SONG = join(process.cwd(), '..', 'crates', 'project', 'tests', 'reference', 'song')
const BASS = '01HF000000000000000000000J'

interface File {
  path: string
  bytes: number[]
}

function readFolder(dir: string, base = ''): File[] {
  return readdirSync(dir).flatMap((name): File[] => {
    const path = join(dir, name)
    const relative = base ? `${base}/${name}` : name
    return statSync(path).isDirectory() ? readFolder(path, relative) : [{ path: relative, bytes: [...readFileSync(path)] }]
  })
}

const text = (s: string) => [...new TextEncoder().encode(s)]
const feedback = (page: Page) => page.getByTestId('feedback')
const master = async (page: Page) => Number(await feedback(page).getAttribute('data-master-peak'))
const send = (page: Page, bytes: number[]) =>
  page.evaluate((b) => (window as unknown as { __fakeMidi: { send(b: number[]): void } }).__fakeMidi.send(b), bytes)
const open = (page: Page, name: string, files: File[]) =>
  page.evaluate(
    ([n, f]) => (window as unknown as { __faulkOpen: (n: string, f: unknown) => Promise<{ status: string; reason?: string }> }).__faulkOpen(n as string, f),
    [name, files] as const,
  )

test('opening a project folder stops the old project, shows the new one and its solo, and refuses a bad folder', async ({ page }) => {
  const { errors } = watch(page)
  await fakeWebMidi(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  const hook = await page.evaluate(() => typeof (window as unknown as { __faulkOpen?: unknown }).__faulkOpen)
  test.skip(hook !== 'function', 'the dispatcher hook exists in the dev server only')
  await page.getByRole('radio', { name: 'E2E Keys' }).check()
  await expect(page.getByTestId('track-header')).toHaveCount(1)

  // A key is held on the old project: it sounds.
  await send(page, [0x90, 69, 127])
  await expect.poll(() => master(page)).toBeGreaterThan(0.05)

  // A folder that is not a project: a notice, the old project and its note stay.
  const song = readFolder(SONG)
  const newer = song.map((f) => (f.path === 'project.json' ? { path: f.path, bytes: text(Buffer.from(f.bytes).toString().replace('"format": 1', '"format": 99')) } : f))
  expect(await open(page, 'newer', newer)).toEqual({ status: 'refused', reason: 'newer_format' })
  await expect(page.getByRole('status').filter({ hasText: 'newer version' })).toBeVisible()
  expect(await open(page, 'broken', [...song.filter((f) => f.path !== 'project.json'), { path: 'project.json', bytes: text('{"format": 1') }])).toEqual({ status: 'refused', reason: 'corrupt' })
  await expect(page.getByRole('status').filter({ hasText: 'damaged or incomplete' })).toBeVisible()
  await expect(page.getByTestId('track-header')).toHaveCount(1)
  // Nothing was touched: the note still sounds (polled over the synth's release) and the keyboard stays selected.
  await page.waitForTimeout(600)
  expect(await master(page)).toBeGreaterThan(0.05)
  await expect(page.getByRole('radio', { name: 'E2E Keys' })).toBeChecked()

  // The reference song with a session that solos its bass: the old note is gone with the old project.
  const session = { path: '.faulk/session.json', bytes: text(JSON.stringify({ selected_track: BASS, solo: [BASS], midi_input: null })) }
  expect(await open(page, 'song', [...song, session])).toEqual({ status: 'opened', name: 'Song "one"' })
  await expect(page.getByTestId('track-header')).toHaveCount(2)
  await expect(page.getByTestId('track-header').first()).toContainText('Bass')
  await expect(page.getByRole('button', { name: 'Solo Bass', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await expect(page.getByRole('button', { name: 'Solo Drums', exact: true })).toHaveAttribute('aria-pressed', 'false')
  await expect.poll(() => master(page), { timeout: 5000 }).toBeLessThan(0.001)
  await expect(page.getByRole('status').filter({ hasText: 'damaged or incomplete' })).toHaveCount(0)

  // The new project plays: a key on the selected track sounds, and the opened project has no history.
  await send(page, [0x80, 69, 0])
  await page.getByRole('radio', { name: 'E2E Keys' }).check()
  await send(page, [0x90, 60, 127])
  await expect.poll(() => master(page), { timeout: 5000 }).toBeGreaterThan(0.001)
  await send(page, [0x80, 60, 0])
  expect(errors).toEqual([])
})
