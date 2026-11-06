import { expect, test } from '@playwright/test'
import { WebSocketServer, type RawData, type WebSocket } from 'ws'
import { PROTOCOL_VERSION } from '../../src/lib/generated/protocol'
import { engineUp, HELPER_HOST, HELPER_PORT, skipWithoutDevHooks, watch } from './support'

// The page reaches the helper by WebSocket only. COEP does not govern WebSockets; this checks that
// probe, pairing and the session still work from the isolated page, against a stand-in helper that
// speaks the protocol on the helper's real address. The real helper is checked by hand (see the PR).
//
// Since BO-1 the page sends the helper its engine streams in binary frames (a snapshot newer than the
// epoch of the hello, then each change; the table of the helper's MIDI input as a message, #598), and runs
// the messages of the helper's MIDI input in its project, sending bytes only for those the helper did not
// play. The stand-in reads the message headers of each stream (`engine::msg`: tag at byte 2, epoch at
// byte 4, payload length at byte 8, each frame padded to 8 bytes).

const CODE = '12345678'
const TOKEN = 'e2e-token'

interface Connection {
  path: string
  origin: string | undefined
}

/** The text of a frame: `ws` hands over a Buffer, a list of fragments or an ArrayBuffer. */
const text = (data: RawData) => (Array.isArray(data) ? Buffer.concat(data) : data instanceof ArrayBuffer ? Buffer.from(data) : data).toString('utf8')

let server: WebSocketServer
const connections: Connection[] = []
/** The JSON messages and the engine streams the page sent on its sessions. */
const messages: unknown[] = []
const streams: Buffer[] = []
/** The open session of the stand-in, to send it `midi.message`s. */
let sessionSocket: WebSocket | null = null
/** The epoch the stand-in's hello announces: its engine outlives pages, it is ahead of the page's core. */
const HELLO_EPOCH = 40

/** `ControlMsg` tags (`engine::msg::Tag`). */
const RESET = 0
const NOTE_ON = 90
const NOTE_OFF = 91

/** The (tag, epoch) of each message of an engine stream. */
function headers(stream: Buffer): { tag: number; epoch: number }[] {
  const out: { tag: number; epoch: number }[] = []
  for (let at = 0; at + 16 <= stream.length;) {
    out.push({ tag: stream.readUInt16LE(at + 2), epoch: stream.readUInt16LE(at + 4) })
    at += 16 + Math.ceil(stream.readUInt32LE(at + 8) / 8) * 8
  }
  return out
}

/** Told each stream as it arrives (the measure of the MIDI round trip). */
let onStream: ((stream: Buffer) => void) | null = null

/** The time (`performance.now()`) the next stream holding a message of `tag` arrives; rejects after 5 s. */
function nextStreamWith(tag: number): Promise<number> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`no stream with tag ${tag}`)), 5000)
    onStream = (stream) => {
      if (!headers(stream).some((h) => h.tag === tag)) return
      clearTimeout(timer)
      onStream = null
      resolve(performance.now())
    }
  })
}

/** Whether a stream since `from` holds a message of `tag`. */
const sent = (tag: number, from = 0) => streams.slice(from).some((stream) => headers(stream).some((h) => h.tag === tag))
/** Whether the stand-in has an audio output (#525): without one its probe says so and it refuses sessions with `audio.no_output`. */
let hasOutput = true
/** Whether the stand-in was shown the project's folder in its dialog (AU-7). */
let folderKnown = false
/** The setup streams of the `export.start`s the stand-in received, decoded from base64 (#319). */
const exportSetups: Buffer[] = []

test.beforeAll(async () => {
  server = new WebSocketServer({ host: HELPER_HOST, port: HELPER_PORT })
  await new Promise<void>((resolve, reject) => {
    server.once('listening', resolve)
    server.once('error', (error) => reject(new Error(`port ${HELPER_PORT} is taken (a helper running?): ${String(error)}`)))
  })
  server.on('connection', (ws: WebSocket, req) => {
    const url = new URL(req.url ?? '/', `http://${HELPER_HOST}`)
    connections.push({ path: url.pathname, origin: req.headers.origin })
    const send = (message: object) => ws.send(JSON.stringify(message))
    if (url.pathname === '/probe') send({ type: 'probe', helper: 'faulk-helper', protocol: PROTOCOL_VERSION, audio_output: hasOutput })
    else if (url.pathname === '/pair') {
      ws.on('message', (data) => {
        const message = JSON.parse(text(data))
        if (message.type === 'pair' && message.code === CODE) send({ type: 'paired', token: TOKEN })
        else send({ type: 'pairing_refused', attempts_left: 2 })
      })
    } else if (url.pathname === '/session' && url.searchParams.get('token') === TOKEN && !hasOutput) {
      send({ type: 'error', code: 'audio.no_output', message: 'the helper has no audio output' })
      ws.close()
    } else if (url.pathname === '/session' && url.searchParams.get('token') === TOKEN) {
      send({
        type: 'hello',
        protocol: PROTOCOL_VERSION,
        sample_rate: 48000,
        estimated_latency_ms: 5,
        epoch: HELLO_EPOCH,
        capabilities: { midi: true, disk: true, export: true },
      })
      send({ type: 'midi.inputs', inputs: [], selected: null })
      sessionSocket = ws
      ws.on('message', (data, isBinary) => {
        if (isBinary) {
          const stream = Array.isArray(data) ? Buffer.concat(data) : Buffer.from(data as Buffer)
          streams.push(stream)
          onStream?.(stream)
          return
        }
        const message = JSON.parse(text(data)) as { type: string; project?: string; name?: string; id?: number; setup?: string; body_frames?: number }
        messages.push(message)
        // The export (#319): the stand-in reads the job's setup stream as the helper decodes it, then answers.
        if (message.type === 'export.start') {
          const setup = Buffer.from(message.setup ?? '', 'base64')
          exportSetups.push(setup)
          const id = message.id ?? 0
          const body = message.body_frames ?? 0
          send({ type: 'export.progress', id, rendered: Math.floor(body / 2), body, tail: false })
          setTimeout(
            () =>
              send({
                type: 'export.done',
                id,
                path: `exports/${message.name}.wav`,
                bytes: 44 + body * 6,
                written: body,
                peak: 0.5,
                clipped: 0,
                reduction: 0,
                silenced: 0,
                tail_cut: false,
                missing: [],
              }),
            200,
          )
        }
        // The project's folder (AU-7): the stand-in knows none until the user shows it in its dialog.
        if (message.type === 'folder.open')
          send({ type: 'folder.state', project: message.project, state: folderKnown ? { status: 'open', name: message.name } : { status: 'unknown' } })
        else if (message.type === 'folder.close') send({ type: 'folder.state', project: null, state: { status: 'closed' } })
        else if (message.type === 'folder.choose') {
          send({ type: 'folder.state', project: message.project, state: { status: 'choosing' } })
          folderKnown = true
          setTimeout(() => send({ type: 'folder.state', project: message.project, state: { status: 'open', name: message.name } }), 300)
        }
      })
    } else if (url.pathname === '/control' && url.searchParams.get('token') === TOKEN) {
      // The page's control link (#608): agents' calls out, their answers in.
      controlSocket = ws
      send({ type: 'control.hello', protocol: PROTOCOL_VERSION })
      ws.on('message', (data) => controlMessages.push(JSON.parse(text(data))))
    } else ws.close()
  })
})

/** The page's control link to the stand-in (#608), and what the page sent on it. */
let controlSocket: WebSocket | null = null
const controlMessages: unknown[] = []

/** Sends an agent's call on the control link and waits for the page's answer. */
async function agentCall(id: number, command: object): Promise<unknown> {
  controlSocket!.send(JSON.stringify({ type: 'agent.call', id, agent: 'e2e-agent', command }))
  let answer: unknown
  await expect
    .poll(() => (answer = controlMessages.find((m) => (m as { type?: string; id?: number }).type === 'agent.result' && (m as { id: number }).id === id)))
    .toBeTruthy()
  return (answer as { result: unknown }).result
}

test.afterAll(async () => {
  for (const client of server.clients) client.terminate()
  await new Promise((resolve) => server.close(resolve))
})

test.beforeEach(() => {
  connections.length = 0
  messages.length = 0
  streams.length = 0
  sessionSocket = null
  controlSocket = null
  controlMessages.length = 0
  onStream = null
  hasOutput = true
  folderKnown = false
  exportSetups.length = 0
})

test('in Boost the page names its folder by a marker it writes there, never a path, and asks the user to show it to the helper (AU-7)', async ({ page }) => {
  const { errors } = watch(page)
  // A real folder: the page's folder dialog hands back a directory of the origin private file system (a `FileSystemDirectoryHandle` like a real one).
  await page.addInitScript(() => {
    const w = window as unknown as { __faulkNoBrowserStorage: boolean; showDirectoryPicker: () => Promise<FileSystemDirectoryHandle> }
    w.__faulkNoBrowserStorage = true
    w.showDirectoryPicker = async () => (await navigator.storage.getDirectory()).getDirectoryHandle('helper-116', { create: true })
  })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  await skipWithoutDevHooks(page)
  const run = (command: object) => page.evaluate((c) => (window as unknown as { __faulkRun(c: object): boolean }).__faulkRun(c), command)
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-save-as').click()
  await page.getByTestId('save-as-field').fill('Song')
  await page.getByTestId('save-as-submit').click()
  await expect(page.getByTestId('save-state')).toHaveText(/^Saved at/)
  await run({ type: 'track.create', kind: 'audio' })
  const id = await page.evaluate(() => (window as unknown as { __faulkView(): { project: { id: string } } }).__faulkView().project.id)
  const track = await page.evaluate(
    () =>
      Object.entries((window as unknown as { __faulkView(): { project: { tracks: Record<string, { kind: string }> } } }).__faulkView().project.tracks).find(
        ([, t]) => t.kind === 'audio',
      )![0],
  )
  await run({ type: 'view.select_track', track })
  await page.getByTestId('panel-clips').click()

  await page.getByTestId('engine-mode').click()
  await page.getByRole('textbox', { name: /code/i }).fill('1234 5678')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await engineUp(page, 'Boost')
  await page.keyboard.press('Escape')

  // The page tells the helper its project and a marker it wrote in `.faulk/` of its folder: no path.
  await expect.poll(() => messages.find((m) => (m as { type?: string }).type === 'folder.open')).toBeTruthy()
  const opened = messages.find((m) => (m as { type?: string }).type === 'folder.open') as { project: string; nonce: string; name: string }
  expect(Object.keys(opened).sort()).toEqual(['name', 'nonce', 'project', 'type'])
  expect(opened).toMatchObject({ project: id, name: 'Song' })
  expect(opened.nonce).toMatch(/^[0-9a-f]{32}$/)
  const marker = await page.evaluate(async () => {
    const song = await (await (await navigator.storage.getDirectory()).getDirectoryHandle('helper-116')).getDirectoryHandle('Song')
    const file = await (await (await song.getDirectoryHandle('.faulk')).getFileHandle('link.json')).getFile()
    return JSON.parse(await file.text()) as { nonce: string }
  })
  expect(marker.nonce).toBe(opened.nonce)

  // The stand-in knows no folder: the Clips panel asks the user to show it; the click asks the helper's dialog.
  await expect(page.getByTestId('audio-boost-folder')).toHaveAttribute('data-key', 'clips.boostShowFolder')
  await page.getByTestId('audio-boost-show-folder').click()
  await expect.poll(() => messages.find((m) => (m as { type?: string }).type === 'folder.choose')).toEqual({ ...opened, type: 'folder.choose' })
  await expect(page.getByTestId('audio-boost-folder')).toHaveAttribute('data-key', 'clips.boostChoosing')
  // The helper opened the folder: nothing more to say.
  await expect(page.getByTestId('audio-boost-folder')).toBeHidden()
  // The project still saves in the page: the marker is no change of the folder.
  const tracksOf = () =>
    page.evaluate(
      () => Object.keys((window as unknown as { __faulkView(): { project: { tracks: Record<string, unknown> } } }).__faulkView().project.tracks).length,
    )
  const before = await tracksOf()
  await run({ type: 'track.create', kind: 'midi' })
  await expect.poll(tracksOf).toBe(before + 1)
  await expect(page.getByTestId('save-state')).toHaveText(/^Saved at/, { timeout: 8000 })
  // Read back in Web mode: a reload plays in the page again and opens the same folder, the change made in Boost included.
  await page.reload()
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  await expect.poll(tracksOf).toBe(before + 1)
  await expect(page.getByTestId('project-name')).toHaveText('Song')
  expect(errors).toEqual([])
})

test('in Boost, with the folder open in the helper, the page hands the export job to the helper and opens no file dialog (#319)', async ({ page }) => {
  const { errors } = watch(page)
  folderKnown = true
  await page.addInitScript(() => {
    const w = window as unknown as {
      __faulkNoBrowserStorage: boolean
      __picks: number
      showDirectoryPicker: () => Promise<FileSystemDirectoryHandle>
      showSaveFilePicker: () => Promise<FileSystemFileHandle>
    }
    w.__faulkNoBrowserStorage = true
    w.__picks = 0
    w.showDirectoryPicker = async () => (await navigator.storage.getDirectory()).getDirectoryHandle('helper-319', { create: true })
    w.showSaveFilePicker = async () => {
      w.__picks++
      throw new DOMException('no', 'AbortError')
    }
  })
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  await skipWithoutDevHooks(page)
  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-save-as').click()
  await page.getByTestId('save-as-field').fill('Song')
  await page.getByTestId('save-as-submit').click()
  await expect(page.getByTestId('save-state')).toHaveText(/^Saved at/)
  // A clip in the arrangement: something to export.
  await page.evaluate(() => {
    const w = window as unknown as { __faulkRun(c: object): boolean; __faulkView(): { project: { tracks: Record<string, { kind: string }> } } }
    const track = Object.entries(w.__faulkView().project.tracks).find(([, t]) => t.kind === 'midi')![0]
    w.__faulkRun({ type: 'clip.create', track, at: { at: 'arrangement', start: 0 }, length: 3840 })
  })

  await page.getByTestId('engine-mode').click()
  await page.getByRole('textbox', { name: /code/i }).fill('1234 5678')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await engineUp(page, 'Boost')
  await page.keyboard.press('Escape')
  await expect.poll(() => messages.find((m) => (m as { type?: string }).type === 'folder.open')).toBeTruthy()

  await page.getByTestId('project-menu').click()
  await page.getByTestId('menu-export').click()
  const dialog = page.getByTestId('export-dialog')
  await expect(dialog.getByTestId('export-folder-note')).toBeVisible()
  await dialog.getByTestId('export-start').click()
  await expect(dialog).toHaveAttribute('data-phase', 'done')
  await expect(dialog.getByTestId('export-status')).toContainText('Saved in the project folder: “exports/Song.wav”')
  // No file dialog; the job went to the helper, and its setup stream is the page's engine stream (a `Reset` first).
  expect(await page.evaluate(() => (window as unknown as { __picks: number }).__picks)).toBe(0)
  const start = messages.find((m) => (m as { type?: string }).type === 'export.start') as Record<string, unknown>
  const id = await page.evaluate(() => (window as unknown as { __faulkView(): { project: { id: string } } }).__faulkView().project.id)
  expect(start).toMatchObject({ project: id, name: 'Song', sample_rate: 48000, bits: 24 })
  expect(Number(start.body_frames)).toBe(96000)
  expect(exportSetups).toHaveLength(1)
  expect(headers(exportSetups[0]!)[0]).toMatchObject({ tag: RESET })
  expect(headers(exportSetups[0]!).length).toBeGreaterThan(1)
  expect(errors).toEqual([])
})

test("a paired page in Web mode runs an agent's call on its control link, with or without the helper's output (#608)", async ({ page }) => {
  const { errors } = watch(page)
  hasOutput = false
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await page.getByTestId('engine-mode').click()
  await page.getByRole('textbox', { name: /code/i }).fill('1234 5678')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await expect(page.getByTestId('helper-no-output')).toBeVisible()
  await page.getByRole('button', { name: 'Stay in Web' }).click()
  await page.keyboard.press('Escape')
  await engineUp(page, 'Web')

  // No output, the page plays itself: an agent's call still reaches it and runs like the user's action.
  await expect.poll(() => controlSocket !== null).toBe(true)
  const tracks = await page.getByTestId('grid-track').count()
  expect(await agentCall(1, { type: 'track.create', kind: 'midi' })).toMatchObject({ status: 'applied' })
  await expect(page.getByTestId('grid-track')).toHaveCount(tracks + 1)
  // Its undo is the user's; a command that needs a click is refused without running.
  expect(await agentCall(2, { type: 'history.undo' })).toMatchObject({ status: 'applied' })
  await expect(page.getByTestId('grid-track')).toHaveCount(tracks)
  expect(await agentCall(3, { type: 'project.save_as' })).toMatchObject({ status: 'unavailable' })
  // The page coming to the front says so on its link.
  await page.evaluate(() => window.dispatchEvent(new Event('focus')))
  await expect.poll(() => controlMessages).toContainEqual({ type: 'control.front' })

  // A reload with the output there: Web mode still (Boost is the user's choice), and the link serves again.
  hasOutput = true
  controlSocket = null
  await page.reload()
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await engineUp(page, 'Web')
  await expect.poll(() => controlSocket !== null).toBe(true)
  const reloaded = await page.getByTestId('grid-track').count()
  expect(await agentCall(4, { type: 'track.create', kind: 'midi' })).toMatchObject({ status: 'applied' })
  await expect(page.getByTestId('grid-track')).toHaveCount(reloaded + 1)
  expect(errors).toEqual([])
})

/** Sends an agent's call on the control link, without waiting: the page answers once the user did (AG-4). */
async function agentCallLater(id: number, command: object): Promise<unknown> {
  controlSocket!.send(JSON.stringify({ type: 'agent.call', id, agent: 'e2e-agent', command }))
  let answer: unknown
  await expect
    .poll(() => (answer = controlMessages.find((m) => (m as { type?: string; id?: number }).type === 'agent.result' && (m as { id: number }).id === id)), {
      timeout: 60_000,
    })
    .toBeTruthy()
  return (answer as { result: unknown }).result
}

test("an agent's destructive call waits for the user's answer in the page: refused, nothing changes; allowed, it runs (AG-4)", async ({ page }) => {
  const { errors } = watch(page)
  hasOutput = false
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await page.getByTestId('engine-mode').click()
  await page.getByRole('textbox', { name: /code/i }).fill('1234 5678')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await expect(page.getByTestId('helper-no-output')).toBeVisible()
  await page.getByRole('button', { name: 'Stay in Web' }).click()
  await page.keyboard.press('Escape')
  await engineUp(page, 'Web')
  await expect.poll(() => controlSocket !== null).toBe(true)

  // An agent edits first: the project has a track the user would lose.
  expect(await agentCall(1, { type: 'track.create', kind: 'midi' })).toMatchObject({ status: 'applied' })
  const tracks = await page.getByTestId('grid-track').count()

  // project.new waits for the question; nothing is answered nor replaced meanwhile.
  const refused = agentCallLater(2, { type: 'project.new', name: 'From the agent' })
  const dialog = page.getByTestId('agent-confirm')
  await expect(dialog).toBeVisible()
  await expect(dialog).toHaveAttribute('role', 'alertdialog')
  await expect(dialog.getByRole('heading')).toHaveText('e2e-agent wants to start a new project')
  await expect(page.getByTestId('agent-confirm-refuse')).toBeFocused()
  await expect(page.getByTestId('agent-confirm-countdown')).toContainText(/Refused automatically in \d+ s\./)
  expect(controlMessages.some((m) => (m as { id?: number }).id === 2)).toBe(false)
  await page.screenshot({ path: test.info().outputPath('agent-confirm.png') })
  await page.getByTestId('agent-confirm-refuse').click()
  expect(await refused).toMatchObject({ status: 'refused', error: { code: 'confirmation.declined' } })
  await expect(dialog).toBeHidden()
  await expect(page.getByTestId('grid-track')).toHaveCount(tracks)

  // Escape refuses too, above another modal dialog of the page (the shortcut list) that stays open behind it.
  await page.getByTestId('shortcuts-open').click()
  await expect(page.getByTestId('shortcuts-dialog')).toBeVisible()
  const escaped = agentCallLater(3, { type: 'project.new' })
  await expect(dialog).toBeVisible()
  await expect(page.getByTestId('agent-confirm-refuse')).toBeFocused()
  await page.keyboard.press('Escape')
  expect(await escaped).toMatchObject({ status: 'refused', error: { code: 'confirmation.declined' } })
  await expect(dialog).toBeHidden()
  await expect(page.getByTestId('shortcuts-dialog')).toBeVisible()
  await page.getByTestId('shortcuts-close').click()
  await expect(page.getByTestId('grid-track')).toHaveCount(tracks)

  // Two at once: one question after the other, the second says it waits.
  const first = agentCallLater(4, { type: 'project.new' })
  const second = agentCallLater(5, { type: 'project.new', name: 'From the agent' })
  await expect(page.getByTestId('agent-confirm-countdown')).toContainText('1 more request is waiting.')
  await page.getByTestId('agent-confirm-refuse').click()
  expect(await first).toMatchObject({ status: 'refused' })
  await expect(page.getByTestId('agent-confirm-countdown')).not.toContainText('waiting')
  // Allowed: the project is replaced, once, with no second question.
  await page.getByTestId('agent-confirm-allow').click()
  expect(await second).toEqual({ status: 'applied', output: { name: 'From the agent' } })
  await expect(dialog).toBeHidden()
  await expect(page.getByTestId('confirm')).toBeHidden()
  await expect(page.getByTestId('grid-track')).toHaveCount(1)
  expect(errors).toEqual([])
})

test('probes, pairs and plays through the helper from the isolated page', async ({ page, baseURL }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  expect(await page.evaluate(() => crossOriginIsolated), 'crossOriginIsolated').toBe(true)
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()

  // Probe answered: the pairing form shows, in the engine dialog (the pill opens it).
  await page.getByTestId('engine-mode').click()
  const code = page.getByRole('textbox', { name: /code/i })
  await expect(code).toBeVisible()

  // A wrong code is refused by the helper, the right one opens a session.
  await code.fill('0000 0000')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await expect(page.getByText('Wrong code. 2 attempts left.')).toBeVisible()
  await code.fill('1234 5678')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await engineUp(page, 'Boost')
  await expect(page.getByTestId('latency')).toContainText('5')
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('engine-dialog')).toBeHidden()
  await expect(page.locator('[aria-modal="true"]')).toHaveCount(0)

  // The helper's engine starts from the page's project: its first stream is a Reset newer than the hello's epoch (BO-1).
  await expect.poll(() => streams.length).toBeGreaterThan(0)
  const snapshot = headers(streams[0])
  expect(snapshot[0]).toEqual({ tag: RESET, epoch: HELLO_EPOCH + 1 })
  expect(snapshot.length).toBeGreaterThan(1)
  // No command crosses the link: the page asks the helper's MIDI inputs with a message for its devices.
  expect(messages.some((m) => (m as { type?: string }).type === 'command')).toBe(false)
  expect(messages).toContainEqual({ type: 'midi.list_inputs' })

  // A played key reaches the helper as engine bytes, in the snapshot's epoch. The dialog gave the focus back to the pill: the letters still play (a push button keeps Space and Enter only).
  await expect(page.getByTestId('engine-mode')).toBeFocused()
  const before = streams.length
  await page.keyboard.down('KeyA')
  await page.keyboard.up('KeyA')
  await expect.poll(() => sent(NOTE_ON, before)).toBe(true)
  expect(
    streams
      .slice(before)
      .flatMap(headers)
      .every((h) => h.epoch === HELLO_EPOCH + 1),
  ).toBe(true)

  // The table of the helper's MIDI input (#598): sent right behind the snapshot, in its epoch.
  type Routing = { type: 'live.routing'; epoch: number; tracks: number[] }
  const routings = () => messages.filter((m): m is Routing => (m as { type?: string }).type === 'live.routing')
  await expect.poll(() => routings().length).toBeGreaterThan(0)
  expect(routings().every((r) => r.epoch === HELLO_EPOCH + 1 && Array.isArray(r.tracks))).toBe(true)

  // A key the helper played with that table runs in the page's project (the key shown) and sends no engine bytes.
  const quiet = streams.length
  sessionSocket!.send(JSON.stringify({ type: 'midi.message', data: [0x90, 67, 100], played: true }))
  sessionSocket!.send(JSON.stringify({ type: 'midi.message', data: [0x80, 67, 0], played: true }))
  // A control change after them is the page's: its bytes are the first stream since, so the notes sent none.
  sessionSocket!.send(JSON.stringify({ type: 'midi.message', data: [0xb0, 123, 0], played: false }))
  await expect.poll(() => streams.length).toBeGreaterThan(quiet)
  expect(sent(NOTE_ON, quiet) || sent(NOTE_OFF, quiet)).toBe(false)

  // A key the helper did not play (no table yet, another epoch) plays through the page's project and comes
  // back as engine bytes. That fallback's page leg, measured: `midi.message` out to the engine stream back.
  const legs: number[] = []
  for (let i = 0; i < 40; i++) {
    for (const [status, tag] of [
      [0x90, NOTE_ON],
      [0x80, NOTE_OFF],
    ]) {
      const back = nextStreamWith(tag)
      const start = performance.now()
      sessionSocket!.send(JSON.stringify({ type: 'midi.message', data: [status, 64, 100], played: false }))
      legs.push((await back) - start)
    }
  }
  legs.sort((a, b) => a - b)
  const at = (q: number) => legs[Math.min(legs.length - 1, Math.floor(q * legs.length))].toFixed(1)
  console.log(
    `helper MIDI through the page: ${legs.length} messages, median ${at(0.5)} ms, p95 ${at(0.95)} ms, max ${at(1)} ms (stand-in helper, Node timestamps on its socket)`,
  )

  // Back to Web, the page's own engine plays again.
  await page.getByTestId('engine-mode').click()
  await page.getByRole('button', { name: 'Back to Web' }).click()
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('engine-dialog')).toBeHidden()
  await engineUp(page, 'Web')
  // The Web engine caught up by snapshot: a key sounds again.
  await page.keyboard.down('KeyA')
  await expect.poll(async () => Number(await page.getByTestId('top-meter').getAttribute('aria-valuenow')), { timeout: 10_000 }).toBeGreaterThan(-60)
  await page.keyboard.up('KeyA')

  const origin = new URL(baseURL!).origin
  // The page's own sockets, by their Origin header. Other pages on the machine (another checkout's
  // dev server, a browser tab) may probe the helper's port too: they are left out, not counted.
  const own = connections.filter((c) => c.origin === origin).map((c) => c.path)
  expect(own).toEqual(expect.arrayContaining(['/probe', '/pair', '/session']))
  expect(errors).toEqual([])
})

test('without an audio output the helper keeps the page in Web mode, says so, and Boost starts once the output is there (#525)', async ({ page }) => {
  const { errors } = watch(page)
  hasOutput = false
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()
  await page.getByTestId('engine-mode').click()
  await page.getByRole('textbox', { name: /code/i }).fill('1234 5678')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()

  // Paired, but no output: the page plays itself, the dialog says why and the pill shows the wait.
  const message = page.getByTestId('helper-no-output')
  await expect(message).toContainText('The helper has no audio output it can use.')
  await expect(page.getByRole('button', { name: 'Stay in Web' })).toBeVisible()
  await expect(page.getByTestId('engine-dialog-mode')).toHaveText('Web (in the browser)')
  await expect(page.getByTestId('engine-waiting')).toBeVisible()
  await expect(page.getByTestId('engine-mode')).toHaveAttribute('aria-label', /Boost waits for an audio output/)
  await page.keyboard.press('Escape')
  // The dialog is gone and the pill has the focus back before a key is played: a key pressed
  // while the dialog still closes is the dialog's, and a held key is not pressed again.
  await expect(page.getByTestId('engine-dialog')).toBeHidden()
  await expect(page.locator('[aria-modal="true"]')).toHaveCount(0)
  await expect(page.getByTestId('engine-mode')).toBeFocused()
  // The Web engine plays meanwhile.
  await page.keyboard.down('KeyA')
  await expect.poll(async () => Number(await page.getByTestId('top-meter').getAttribute('aria-valuenow')), { timeout: 10_000 }).toBeGreaterThan(-60)
  await page.keyboard.up('KeyA')

  // The output appears: the page switches to Boost by itself, without a click.
  hasOutput = true
  await engineUp(page, 'Boost')
  await expect(page.getByTestId('engine-waiting')).toHaveCount(0)
  await page.keyboard.down('KeyA')
  await page.keyboard.up('KeyA')
  await expect.poll(() => sent(NOTE_ON)).toBe(true)
  expect(errors).toEqual([])
})

test('the French page says the helper has no audio output', async ({ page }) => {
  hasOutput = false
  await page.addInitScript(() => localStorage.setItem('faulk.lang', 'fr'))
  await page.goto('/app')
  await page.getByTestId('engine-state').getByRole('button', { name: 'Démarrer l’audio' }).click()
  await page.getByTestId('engine-mode').click()
  await page.getByRole('textbox', { name: /code/i }).fill('1234 5678')
  await page.getByRole('button', { name: 'Appairer', exact: true }).click()
  await expect(page.getByTestId('helper-no-output')).toContainText('Le helper n’a pas de sortie audio utilisable.')
  await page.getByRole('button', { name: 'Rester en Web' }).click()
  await expect(page.getByTestId('helper-no-output')).toHaveCount(0)
  await expect(page.getByTestId('engine-waiting')).toHaveCount(0)
})
