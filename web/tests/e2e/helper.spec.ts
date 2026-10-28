import { expect, test } from '@playwright/test'
import { WebSocketServer, type RawData, type WebSocket } from 'ws'
import { PROTOCOL_VERSION } from '../../src/lib/generated/protocol'
import { HELPER_HOST, HELPER_PORT, watch } from './support'

// The page reaches the helper by WebSocket only. COEP does not govern WebSockets; this checks that
// probe, pairing and the session still work from the isolated page, against a stand-in helper that
// speaks the protocol on the helper's real address. The real helper is checked by hand (see the PR).

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
const commands: unknown[] = []

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
    if (url.pathname === '/probe') send({ type: 'probe', protocol: PROTOCOL_VERSION })
    else if (url.pathname === '/pair') {
      ws.on('message', (data) => {
        const message = JSON.parse(text(data))
        if (message.type === 'pair' && message.code === CODE) send({ type: 'paired', token: TOKEN })
        else send({ type: 'pairing_refused', attempts_left: 2 })
      })
    } else if (url.pathname === '/session' && url.searchParams.get('token') === TOKEN) {
      send({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000, estimated_latency_ms: 5 })
      send({ type: 'midi.inputs', inputs: [], selected: null })
      ws.on('message', (data) => commands.push(JSON.parse(text(data))))
    } else ws.close()
  })
})

test.afterAll(async () => {
  for (const client of server.clients) client.terminate()
  await new Promise((resolve) => server.close(resolve))
})

test.beforeEach(() => {
  connections.length = 0
  commands.length = 0
})

test('probes, pairs and plays through the helper from the isolated page', async ({ page, baseURL }) => {
  const { errors } = watch(page)
  await page.goto('/app')
  expect(await page.evaluate(() => crossOriginIsolated), 'crossOriginIsolated').toBe(true)
  await page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()

  // Probe answered: the pairing form shows.
  const code = page.getByRole('textbox', { name: /code/i })
  await expect(code).toBeVisible()

  // A wrong code is refused by the helper, the right one opens a session.
  await code.fill('0000 0000')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await expect(page.getByText('Wrong code. 2 attempts left.')).toBeVisible()
  await code.fill('1234 5678')
  await page.getByRole('button', { name: 'Pair', exact: true }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Boost')
  await expect(page.getByTestId('latency')).toContainText('5')

  // Commands reach the helper over the session.
  await page.keyboard.down('KeyA')
  await page.keyboard.up('KeyA')
  await expect.poll(() => commands.some((c) => (c as { command?: { type?: string } }).command?.type === 'live.input_note_on')).toBe(true)

  // Back to Web, the page's own engine plays again.
  await page.getByRole('button', { name: 'Back to Web' }).click()
  await expect(page.getByTestId('engine-mode')).toContainText('Web')
  // The Web engine caught up by snapshot: a key sounds again.
  await page.keyboard.down('KeyA')
  await expect.poll(async () => Number(await page.getByRole('meter').getAttribute('aria-valuenow')), { timeout: 10_000 }).toBeGreaterThan(-60)
  await page.keyboard.up('KeyA')

  const origin = new URL(baseURL!).origin
  // The page's own sockets, by their Origin header. Other pages on the machine (another checkout's
  // dev server, a browser tab) may probe the helper's port too: they are left out, not counted.
  const own = connections.filter((c) => c.origin === origin).map((c) => c.path)
  expect(own).toEqual(expect.arrayContaining(['/probe', '/pair', '/session']))
  expect(errors).toEqual([])
})
