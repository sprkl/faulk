import { expect, test, type Page } from '@playwright/test'
import { WebSocketServer, type WebSocket } from 'ws'
import { PROTOCOL_VERSION } from '../../src/lib/generated/protocol'
import { engineUp, HELPER_HOST, HELPER_PORT, watch } from './support'

// A big project's snapshot in the real page (#245): a project of several thousand clips, whose
// snapshot is longer than the engine takes at once (its rt ring and its prepare queue, 8 192
// messages), is built by commands, then the page switches to Boost (a stand-in helper, as in
// `helper.spec.ts`) and back to Web: the snapshot goes to the Web engine while its audio context
// is suspended and the prepare Worker's queue sends it in pieces as the worklet consumes it. No
// stream is refused, and the time from the click on "Back to Web" to the sound of a scene launched
// right after it (the launch waits behind the snapshot) is printed, for the empty project and the
// big one. The dispatcher's dev hook (`__faulkRun`) builds the project: dev project only.

const CODE = '12345678'
const TOKEN = 'e2e-big-project-token'
const BAR = 3840

let server: WebSocketServer

test.beforeAll(async () => {
  server = new WebSocketServer({ host: HELPER_HOST, port: HELPER_PORT })
  await new Promise<void>((resolve, reject) => {
    server.once('listening', resolve)
    server.once('error', (error) => reject(new Error(`port ${HELPER_PORT} is taken (a helper running?): ${String(error)}`)))
  })
  server.on('connection', (ws: WebSocket, req) => {
    const url = new URL(req.url ?? '/', `http://${HELPER_HOST}`)
    const send = (message: object) => ws.send(JSON.stringify(message))
    if (url.pathname === '/probe') send({ type: 'probe', protocol: PROTOCOL_VERSION })
    else if (url.pathname === '/pair') ws.on('message', () => send({ type: 'paired', token: TOKEN }))
    else if (url.pathname === '/session' && url.searchParams.get('token') === TOKEN) {
      send({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000, estimated_latency_ms: 5 })
      send({ type: 'midi.inputs', inputs: [], selected: null })
    } else ws.close()
  })
})

test.afterAll(async () => {
  for (const client of server.clients) client.terminate()
  await new Promise((resolve) => server.close(resolve))
})

type Hooks = {
  __faulkRun(command: unknown): boolean
  __faulkView(): { project: { tracks: Record<string, { clips: Record<string, unknown> }>; scenes: Record<string, unknown> } }
}

/** `tracks` tracks of `scenes` grid clips and `bars` arrangement clips, one note each: the first track filled, then duplicated. */
function build(page: Page, tracks: number, scenes: number, bars: number) {
  return page.evaluate(
    ([tracks, scenes, bars, BAR]) => {
      const w = window as unknown as Hooks
      const run = (command: unknown) => {
        if (!w.__faulkRun(command)) throw new Error(`refused: ${JSON.stringify(command)}`)
      }
      const track = Object.keys(w.__faulkView().project.tracks)[0]
      const clipsBefore = new Set(Object.keys(w.__faulkView().project.tracks[track].clips))
      const scenesBefore = new Set(Object.keys(w.__faulkView().project.scenes))
      for (let i = 0; i < scenes; i++) run({ type: 'scene.create' })
      const sceneIds = Object.keys(w.__faulkView().project.scenes).filter((s) => !scenesBefore.has(s))
      for (const scene of sceneIds) run({ type: 'clip.create', track, at: { at: 'slot', scene }, length: BAR })
      for (let bar = 0; bar < bars; bar++) run({ type: 'clip.create', track, at: { at: 'arrangement', start: bar * BAR }, length: BAR })
      for (const clip of Object.keys(w.__faulkView().project.tracks[track].clips).filter((c) => !clipsBefore.has(c)))
        run({ type: 'clip.notes.add', clip, notes: [{ start: 0, pitch: 60, length: BAR / 2, velocity: 100, cents: 0, chance: 100, repeat: 1 }] })
      for (let i = 1; i < tracks; i++) run({ type: 'track.duplicate', track })
      const view = w.__faulkView().project
      return Object.values(view.tracks).reduce((n, t) => n + Object.keys(t.clips).length, 0)
    },
    [tracks, scenes, bars, BAR] as const,
  )
}

const masterPeak = async (page: Page) => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))

/** Boost, then back to Web with a scene launched at once: ms from the click to the master's sound. */
async function switchAndPlay(page: Page): Promise<number> {
  await page.getByTestId('engine-mode').click()
  await page.getByRole('button', { name: 'Switch to Boost' }).click()
  await engineUp(page, 'Boost')
  // Silent before the switch: the Web engine is suspended and nothing was launched.
  await expect.poll(() => masterPeak(page)).toBeLessThan(0.001)
  const back = page.getByRole('button', { name: 'Back to Web' })
  const start = Date.now()
  await back.click()
  await page.evaluate(() => {
    const w = window as unknown as Hooks
    const scene = Object.keys(w.__faulkView().project.scenes)[0]
    w.__faulkRun({ type: 'launch.scene', scene })
  })
  await expect.poll(() => masterPeak(page), { timeout: 20_000, intervals: [5] }).toBeGreaterThan(0.01)
  const ms = Date.now() - start
  await page.keyboard.press('Escape')
  await engineUp(page, 'Web')
  await page.evaluate(() => (window as unknown as Hooks).__faulkRun({ type: 'transport.stop' }))
  return ms
}

test('a project of thousands of clips switches Boost to Web without a refused stream, and the time to sound is printed', async ({ page }) => {
  test.setTimeout(180_000)
  const { errors } = watch(page)
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  const hook = await page.evaluate(() => typeof (window as unknown as { __faulkRun?: unknown }).__faulkRun)
  test.skip(hook !== 'function', 'the dispatcher hook exists in the dev server only')

  // Pair once with the stand-in helper.
  await page.getByTestId('engine-mode').click()
  await page.getByRole('textbox', { name: /code/i }).fill(CODE)
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await engineUp(page, 'Boost')
  await page.getByRole('button', { name: 'Back to Web' }).click()
  await page.keyboard.press('Escape')
  await engineUp(page, 'Web')

  // The smallest project that plays: one scene, one clip.
  expect(await build(page, 1, 1, 0)).toBe(1)
  const small = await switchAndPlay(page)

  const start = Date.now()
  const clips = await build(page, 48, 63, 64)
  const built = Date.now() - start
  expect(clips).toBeGreaterThanOrEqual(6000)
  const big = await switchAndPlay(page)
  console.log(`big-project: one clip, Back to Web to sound ${small} ms; ${clips} clips (built in ${built} ms), Back to Web to sound ${big} ms`)
  // A loose ceiling: a stalled queue never sounds (the poll fails first), a gross regression is caught here.
  expect(big).toBeLessThan(10_000)
  expect(errors).toEqual([])
})
