import { expect, test, type Page } from '@playwright/test'
import { engineUp, watch } from './support'

// #735: where the browser keeps no projects (no OPFS), a `.faulkpack` is read whole by the page. The banner shows the
// import with its Cancel button, a cancel stops the read and changes nothing, and the next import still opens its project.
// The read is slowed (a stand-in for a pack of 192 MiB) so that the click lands during it.

type Hooks = {
  __faulkRun(c: object): boolean
  __faulkView(): { project: { id: string; tracks: Record<string, unknown> } }
}
const projectId = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__faulkView().project.id)
const trackCount = (page: Page) => page.evaluate(() => Object.keys((window as unknown as Hooks).__faulkView().project.tracks).length)

async function startAudio(page: Page) {
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page)
}

/** Chooses the pack at `path` through the project menu and the file chooser. */
async function importPack(page: Page, path: string) {
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-import-pack').click()
  await (await chooser).setFiles(path)
}

test('without browser storage, a cancel during the whole-file read stops the import and leaves the project alone', async ({ browser }, testInfo) => {
  test.skip(test.info().project.name !== 'dev', 'dev only')
  // A pack made in a page that has the browser's storage.
  const maker = await browser.newPage()
  await startAudio(maker)
  await maker.evaluate(() => (window as unknown as Hooks).__faulkRun({ type: 'track.create', kind: 'midi' }))
  const packed = await projectId(maker)
  await maker.evaluate(() => void ((window as unknown as { showSaveFilePicker?: unknown }).showSaveFilePicker = undefined))
  const download = maker.waitForEvent('download')
  await maker.getByTestId('project-menu').click()
  await maker.getByTestId('menu-export-pack').click()
  const pack = testInfo.outputPath('song.faulkpack')
  await (await download).saveAs(pack)
  await maker.close()

  // A page with no OPFS, and a read that takes a while: the file arrives in small pieces, 80 ms apart.
  const page = await browser.newPage()
  const { errors } = watch(page)
  await page.addInitScript(() => {
    Object.defineProperty(navigator.storage, 'getDirectory', { value: undefined, configurable: true })
    const stream = File.prototype.stream
    File.prototype.stream = function (this: File) {
      const reader = stream.call(this).getReader()
      let held: Uint8Array = new Uint8Array(0)
      return new ReadableStream<Uint8Array<ArrayBuffer>>({
        async pull(controller) {
          await new Promise((resolve) => setTimeout(resolve, 80))
          if (held.length === 0) {
            const next = await reader.read()
            if (next.done) return controller.close()
            held = next.value
          }
          const piece = held.slice(0, 512)
          held = held.slice(512)
          controller.enqueue(piece as Uint8Array<ArrayBuffer>)
        },
        cancel: () => reader.cancel(),
      })
    } as typeof File.prototype.stream
  })
  await startAudio(page)
  const before = await projectId(page)
  const tracks = await trackCount(page)
  expect(before).not.toBe(packed)

  // Cancelled during the read: the banner is there with its button, the project is the same.
  await importPack(page, pack)
  await expect(page.getByTestId('import-banner')).toBeVisible()
  await page.getByTestId('import-cancel').click()
  await expect(page.getByTestId('storage-notice')).toContainText('Import cancelled')
  await expect(page.getByTestId('import-banner')).toHaveCount(0)
  expect(await projectId(page)).toBe(before)
  expect(await trackCount(page)).toBe(tracks)
  await page.getByTestId('storage-notice').getByRole('button').last().click()

  // Cancelled through the command, the way an agent does: the same.
  await importPack(page, pack)
  await expect(page.getByTestId('import-banner')).toBeVisible()
  await page.evaluate(() => (window as unknown as Hooks).__faulkRun({ type: 'project.cancel_import_pack' }))
  await expect(page.getByTestId('storage-notice')).toContainText('Import cancelled')
  expect(await projectId(page)).toBe(before)
  await page.getByTestId('storage-notice').getByRole('button').last().click()

  // Not cancelled: the pack opens.
  await importPack(page, pack)
  await expect.poll(() => projectId(page), { timeout: 60_000 }).toBe(packed)
  expect(errors).toEqual([])
  await page.close()
})
