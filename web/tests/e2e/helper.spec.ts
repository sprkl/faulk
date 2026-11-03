import { expect, test } from '@playwright/test'
import { WebSocketServer, type RawData, type WebSocket } from 'ws'
import { PROTOCOL_VERSION } from '../../src/lib/generated/protocol'
import { engineUp, HELPER_HOST, HELPER_PORT, watch } from './support'

// The page reaches the helper by WebSocket only. COEP does not govern WebSockets; this checks that
// probe, pairing and the session still work from the isolated page, against a stand-in helper that
// speaks the protocol on the helper's real address. The real helper is checked by hand (see the PR).
//
// Since BO-1 the page sends the helper its engine streams in binary frames (a snapshot newer than the
// epoch of the hello, then each change), and plays the messages of the helper's MIDI input through its
// project. The stand-in reads the message headers of each stream (`engine::msg`: tag at byte 2, epoch at
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
        capabilities: { midi: true, disk: false },
      })
      send({ type: 'midi.inputs', inputs: [], selected: null })
      sessionSocket = ws
      ws.on('message', (data, isBinary) => {
        if (isBinary) {
          const stream = Array.isArray(data) ? Buffer.concat(data) : Buffer.from(data as Buffer)
          streams.push(stream)
          onStream?.(stream)
        } else messages.push(JSON.parse(text(data)))
      })
    } else ws.close()
  })
})

test.afterAll(async () => {
  for (const client of server.clients) client.terminate()
  await new Promise((resolve) => server.close(resolve))
})

test.beforeEach(() => {
  connections.length = 0
  messages.length = 0
  streams.length = 0
  sessionSocket = null
  onStream = null
  hasOutput = true
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

  // A key of the helper's MIDI input plays through the page's project and comes back as engine bytes.
  // The page's leg of that round trip, measured: `midi.message` out to the engine stream back.
  const legs: number[] = []
  for (let i = 0; i < 40; i++) {
    for (const [status, tag] of [
      [0x90, NOTE_ON],
      [0x80, NOTE_OFF],
    ]) {
      const back = nextStreamWith(tag)
      const start = performance.now()
      sessionSocket!.send(JSON.stringify({ type: 'midi.message', data: [status, 64, 100] }))
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
