import { spawn, spawnSync, type ChildProcess } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { connect } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { engineUp, HELPER_PORT, watch } from './support'

// AG-5 (#101): an agent builds a song from start to end. A scripted MCP client (this file) speaks to the real helper
// over its stdio bridge (`faulk-helper mcp`, what Claude Code runs), the helper relays each tool call to the page that
// owns the project (its control link, #608: Web mode, the helper started without an audio output and without a tray),
// and the page runs it through its dispatcher as `agent:<client name>`. The agent starts a new project (the user allows
// it in the page, AG-4), makes three MIDI tracks (a synth with a filter, drums, a synth with an echo) and an audio
// track, two scenes with a clip of each track in both, a lane on the filter's cutoff, launches a scene, then places
// the second scene and the audio clip in the arrangement and plays it. The page meters each track (the mixer's peak
// readouts, by track id) in both passes, and the user exports the song: the file is not silent and the engine's output
// protection silenced no frame for a non-finite sample (no NaN or infinity reached the master).
//
// What an agent cannot do alone, the test does as the user: allow `project.new`, and choose the audio file. A file
// dialog needs a click (`clip.choose_audio`), so the file is imported from the Clips panel; the agent then makes its
// clips with `clip.import_audio` and the file's hash, which `clip.get` gives. The export is the user's too
// (`project.export_audio` opens a save dialog). No dev hook: the test runs on the production build as well.
//
// The helper is the debug build of `crates/helper`, built here by cargo (once per run; CI installs the ALSA headers
// it links on Linux), on the checkout's helper port, with its own config folder. Only the processes started here are
// stopped.

test.describe.configure({ mode: 'serial', timeout: 240_000 })

const ROOT = fileURLToPath(new URL('../../../', import.meta.url))
const WAV = join(ROOT, 'crates/audio-import/tests/fixtures/sine-s16.wav')
/** The MCP client's name: the history's author is `agent:<name>`. */
const CLIENT = 'faulk-e2e-agent'
const BAR = 3840

let exe = ''
let config = ''
let helper: ChildProcess | null = null

/** Builds the debug helper and gives its executable (cargo's own report of the artifact). */
function buildHelper(): string {
  const out = spawnSync('cargo', ['build', '-p', 'helper', '--bin', 'faulk-helper', '--message-format=json-render-diagnostics'], {
    cwd: ROOT,
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'inherit'],
  })
  if (out.status !== 0) throw new Error(`cargo build -p helper failed (${out.status})`)
  for (const line of out.stdout.split('\n')) {
    if (!line.startsWith('{')) continue
    const message = JSON.parse(line) as { reason?: string; target?: { name?: string }; executable?: string | null }
    if (message.reason === 'compiler-artifact' && message.target?.name === 'faulk-helper' && message.executable) return message.executable
  }
  throw new Error('cargo did not report the faulk-helper executable')
}

/** The helper's environment: its config folder, no audio output, no tray, no update feed. */
const helperEnv = () => {
  const env: NodeJS.ProcessEnv = { ...process.env, FAULK_HELPER_CONFIG_DIR: config, FAULK_HELPER_TEST_NO_AUDIO: '1', FAULK_HELPER_TEST_NO_TRAY: '1' }
  delete env.FAULK_UPDATE_URL
  return env
}

test.beforeAll(async () => {
  test.setTimeout(900_000) // a cold build of the helper
  exe = buildHelper()
  config = mkdtempSync(join(tmpdir(), 'faulk-agent-e2e-'))
  // The user's consent to agents: the MCP token (`faulk-helper mcp-token`), in the helper's config folder.
  const token = spawnSync(exe, ['mcp-token'], { env: helperEnv(), encoding: 'utf8' })
  expect(token.status, token.stderr).toBe(0)
  expect(token.stdout).toContain('MCP token:')
  const origin = new URL(String(test.info().project.use.baseURL)).origin
  helper = spawn(exe, ['--port', String(HELPER_PORT), '--allow-origin', origin], { env: helperEnv(), stdio: ['ignore', 'ignore', 'inherit'] })
  // Up once it serves its port and has written its session token (the pairing the page will hold).
  await expect
    .poll(
      async () => {
        if (helper?.exitCode !== null) throw new Error(`the helper stopped (${helper?.exitCode})`)
        if (!existsSync(join(config, 'session-token'))) return false
        return new Promise<boolean>((resolve) => {
          const socket = connect(HELPER_PORT, '127.0.0.1', () => {
            socket.end()
            resolve(true)
          })
          socket.on('error', () => resolve(false))
        })
      },
      { timeout: 60_000 },
    )
    .toBe(true)
})

test.afterAll(() => {
  helper?.kill()
  helper = null
  if (config) rmSync(config, { recursive: true, force: true })
})

/** A JSON-RPC client over the helper's MCP stdio bridge, one line per message. */
class Mcp {
  private next = 1
  private readonly waiting = new Map<number, (message: { result?: unknown; error?: { message: string } }) => void>()
  private constructor(private readonly bridge: ChildProcess) {
    createInterface({ input: bridge.stdout! }).on('line', (line) => {
      let message: { id?: number; result?: unknown; error?: { message: string } }
      try {
        message = JSON.parse(line) as typeof message
      } catch {
        // Not a JSON-RPC message: the bridge speaks only JSON on its stdout, so show it rather than fail elsewhere.
        console.log(`MCP bridge, not JSON: ${line}`)
        return
      }
      if (typeof message.id === 'number') this.waiting.get(message.id)?.(message)
    })
  }

  static async connect(): Promise<Mcp> {
    const bridge = spawn(exe, ['mcp', '--port', String(HELPER_PORT)], { env: helperEnv(), stdio: ['pipe', 'pipe', 'inherit'] })
    const mcp = new Mcp(bridge)
    await mcp.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: CLIENT, version: '1' } })
    mcp.send({ jsonrpc: '2.0', method: 'notifications/initialized' })
    return mcp
  }

  private send(message: object) {
    this.bridge.stdin!.write(`${JSON.stringify(message)}\n`)
  }

  request(method: string, params: object): Promise<unknown> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`MCP ${method}: no answer within 60 s`)), 60_000)
      this.waiting.set(id, (message) => {
        clearTimeout(timer)
        this.waiting.delete(id)
        if (message.error) reject(new Error(`MCP ${method}: ${message.error.message}`))
        else resolve(message.result)
      })
      this.send({ jsonrpc: '2.0', id, method, params })
    })
  }

  /** Calls a tool: its structured output when it applied, else the error body (`code`, `message`, `error`). */
  async raw(tool: string, args: object = {}): Promise<{ ok: boolean; body: Record<string, unknown> }> {
    const result = (await this.request('tools/call', { name: tool, arguments: args })) as {
      isError?: boolean
      structuredContent?: Record<string, unknown>
      content: { type: string; text: string }[]
    }
    const text = result.content[0]?.text ?? ''
    if (result.isError) {
      try {
        return { ok: false, body: JSON.parse(text) as Record<string, unknown> }
      } catch {
        return { ok: false, body: { code: 'not-json', message: text } }
      }
    }
    return { ok: true, body: result.structuredContent ?? {} }
  }

  /** Calls a tool that must apply. */
  async call<T = Record<string, unknown>>(tool: string, args: object = {}): Promise<T> {
    const { ok, body } = await this.raw(tool, args)
    if (!ok) throw new Error(`${tool} ${JSON.stringify(args)} failed: ${JSON.stringify(body)}`)
    return body as T
  }

  close() {
    this.bridge.stdin!.end()
    this.bridge.kill()
  }
}

const startAudio = (page: Page) => page.getByTestId('engine-state').getByRole('button', { name: 'Start audio' }).click()

/** Each track's peak readout in the mixer, by track id, in dB (`-Infinity` for none). */
async function peaks(page: Page, tracks: string[]): Promise<Record<string, number>> {
  const out: Record<string, number> = {}
  for (const id of tracks) {
    const text = (await page.locator(`[data-testid=mixer-strip][data-track="${id}"] [data-testid=mixer-meter-peak]`).textContent())!.trim()
    out[id] = text === '−∞' ? -Infinity : Number(text.replace('−', '-').replace('+', ''))
  }
  return out
}

/** A track's readout above this level sounds: the quietest of the four (the echoed synth) reads about -20 dB. */
const AUDIBLE_DB = -40

/** Waits until every track's peak readout is above `AUDIBLE_DB`, and prints them. */
async function everyTrackSounds(page: Page, tracks: string[], what: string) {
  await expect.poll(async () => Object.values(await peaks(page, tracks)).every((db) => db > AUDIBLE_DB), { timeout: 20_000 }).toBe(true)
  const read = await peaks(page, tracks)
  console.log(`peak per track, ${what}: ${tracks.map((id) => `${read[id]!.toFixed(1)} dB`).join(', ')} (Lead, Drums, Echoes, Audio)`)
}

/** Clears every track's peak readout (a click on it, as the user would). */
async function resetPeaks(page: Page, tracks: string[]) {
  for (const id of tracks) await page.locator(`[data-testid=mixer-strip][data-track="${id}"] [data-testid=mixer-meter-peak]`).click()
}

/** Reads a WAV of the OPFS: its frame count and peak (24 bits). */
const readWav = (page: Page, name: string) =>
  page.evaluate(async (file) => {
    const view = new DataView(await (await (await (await navigator.storage.getDirectory()).getFileHandle(file)).getFile()).arrayBuffer())
    const frames = (view.byteLength - 44) / 6
    let peak = 0
    for (let at = 44; at + 3 <= view.byteLength; at += 3) peak = Math.max(peak, Math.abs((view.getInt32(at - 1, true) >> 8) / 8388608))
    return { bits: view.getUint16(34, true), frames, peak }
  }, name)

test('an agent builds a song over MCP: tracks, scenes, clips, a filter lane, a launched scene, the arrangement; every track sounds, no NaN', async ({
  page,
}) => {
  const { errors } = watch(page)
  const session = readFileSync(join(config, 'session-token'), 'utf8').trim()
  // The page is paired with this helper: it holds its session token, as after a pairing.
  await page.addInitScript((token) => localStorage.setItem('faulk.helperToken', token), session)
  // The export's save dialog cannot be driven headless: it hands back a file of the origin private file system.
  await page.addInitScript(() => {
    const w = window as unknown as { __picks: string[]; showSaveFilePicker: (o: { suggestedName: string }) => Promise<FileSystemFileHandle> }
    w.__picks = []
    w.showSaveFilePicker = async (options) => {
      w.__picks.push(options.suggestedName)
      return (await navigator.storage.getDirectory()).getFileHandle(options.suggestedName, { create: true })
    }
  })
  await page.setViewportSize({ width: 1440, height: 900 })
  await page.goto('/app')
  await startAudio(page)
  await engineUp(page, 'Web')

  const mcp = await Mcp.connect()
  try {
    const listed = (await mcp.request('tools/list', {})) as { tools: { name: string }[] }
    for (const tool of ['project_new', 'track_create', 'clip_import_audio', 'automation_lane_create', 'launch_scene', 'arrangement_place_scene'])
      expect(listed.tools.map((t) => t.name)).toContain(tool)
    // The control link opens once the page has probed the helper: until then a call answers `page.absent`.
    await expect.poll(async () => (await mcp.raw('project_get')).ok, { timeout: 30_000 }).toBe(true)

    // A new project replaces the open one: the call waits for the user's answer in the page (AG-4).
    const fresh = mcp.call('project_new')
    await expect(page.getByTestId('agent-confirm')).toBeVisible()
    await page.getByTestId('agent-confirm-allow').click()
    await fresh
    await expect(page.getByTestId('agent-confirm')).toBeHidden()

    // Track 1: the new project's synth, with a filter. Track 2: drums. Track 3: a synth with an echo. Track 4: audio.
    const [first] = (await mcp.call<{ tracks: { id: string; kind: string }[] }>('track_list')).tracks
    expect(first!.kind).toBe('midi')
    const lead = first!.id
    await mcp.call('track_rename', { track: lead, name: 'Lead' })
    const filter = (await mcp.call<{ device: string }>('device_insert', { track: lead, kind: 'faulk.filter', index: 0 })).device
    const drums = (await mcp.call<{ track: string }>('track_create', { kind: 'midi', instrument: 'faulk.drums', name: 'Drums' })).track
    const echoes = (await mcp.call<{ track: string }>('track_create', { kind: 'midi', instrument: 'faulk.osc_synth', name: 'Echoes' })).track
    await mcp.call('device_insert', { track: echoes, kind: 'faulk.echo', index: 0 })
    const audio = (await mcp.call<{ track: string }>('track_create', { kind: 'audio', name: 'Audio' })).track
    const tracks = [lead, drums, echoes, audio]

    // The user chooses the file (a file dialog needs a click): the agent selects the track, the user imports from its Clips panel.
    await mcp.call('view_select_track', { track: audio })
    await page.getByTestId('panel-clips').click()
    const chooser = page.waitForEvent('filechooser')
    await page.getByTestId('audio-import-button').click()
    await (await chooser).setFiles(WAV)
    await expect(page.getByTestId('pool-clip')).toHaveCount(1)
    const pooled = (await mcp.call<{ clips: { id: string; kind: string }[] }>('track_get', { track: audio })).clips.find((c) => c.kind === 'audio')!
    const media = (await mcp.call<{ content: { media: { hash: string } } }>('clip_get', { clip: pooled.id })).content.media.hash
    expect(media).toMatch(/^[0-9a-f]{64}$/)

    // Two scenes, a clip of each track in both.
    const scenes: string[] = (await mcp.call<{ scenes: { id: string }[] }>('scene_list')).scenes.map((s) => s.id)
    while (scenes.length < 2) scenes.push((await mcp.call<{ scene: string }>('scene_create', { name: `Scene ${scenes.length + 1}` })).scene)
    const parts: [string, (scene: number) => { start: number; length: number; pitch: number; velocity: number }[]][] = [
      [lead, (s) => [0, 1, 2, 3].map((i) => ({ start: i * 960, length: 900, pitch: [57, 60, 64, 67][i]! + s * 2, velocity: 100 }))],
      [
        drums,
        () =>
          [0, 1, 2, 3].flatMap((i) => [
            { start: i * 960, length: 240, pitch: i % 2 ? 38 : 36, velocity: 110 },
            { start: i * 960 + 480, length: 120, pitch: 42, velocity: 80 },
          ]),
      ],
      [
        echoes,
        (s) => [
          { start: 0, length: 480, pitch: 72 + s * 3, velocity: 100 },
          { start: 1920, length: 480, pitch: 76, velocity: 100 },
        ],
      ],
    ]
    for (const [scene, at] of scenes.slice(0, 2).entries()) {
      for (const [track, notes] of parts) {
        const { clip } = await mcp.call<{ clip: string }>('clip_create', { track, at: { at: 'slot', scene: at }, length: BAR })
        await mcp.call('clip_notes_add', { clip, notes: notes(scene) })
      }
      const { clip } = await mcp.call<{ clip: string }>('clip_import_audio', { track: audio, media, at: { at: 'slot', scene: at } })
      // The file lasts half a second: the clip loops over it in the grid.
      const { content } = await mcp.call<{ content: { frames: number } }>('clip_get', { clip })
      await mcp.call('clip_audio_set_loop', { clip, looping: { start: 0, end: content.frames } })
    }

    // A lane on the filter's cutoff: closed at the start, open at bar 2.
    const { lane } = await mcp.call<{ lane: string }>('automation_lane_create', { track: lead, target: { device: { device: filter, param: 'cutoff' } } })
    await mcp.call('automation_points_add', {
      lane,
      points: [
        { tick: 0, value: 300 },
        { tick: BAR, value: 8000 },
      ],
    })

    // Every track sounds while the first scene plays.
    await page.getByTestId('nav-mixer').click()
    await expect(page.getByTestId('mixer-strip')).toHaveCount(4)
    await mcp.call('launch_scene', { scene: scenes[0] })
    await everyTrackSounds(page, tracks, 'the first scene, launched')
    await mcp.call('launch_stop_all')
    await mcp.call('transport_stop')

    // The arrangement: the second scene at bar 1 (copies of its MIDI clips), the audio clip there too; back to it, play.
    await mcp.call('arrangement_place_scene', { scene: scenes[1], start: 0 })
    await mcp.call('clip_import_audio', { track: audio, media, at: { at: 'arrangement', start: 0 }, name: 'Sine' })
    await mcp.call('launch_back_to_arrangement')
    await mcp.call('transport_seek', { tick: 0 })
    // The scene's sound (and the echo's tail) dies out, then the readouts are cleared: what they show next is the arrangement's.
    const master = async () => Number(await page.getByTestId('feedback').getAttribute('data-master-peak'))
    await expect.poll(master, { timeout: 15_000 }).toBeLessThan(0.0005)
    await resetPeaks(page, tracks)
    expect(Object.values(await peaks(page, tracks)).every((db) => db === -Infinity)).toBe(true)
    await mcp.call('transport_play')
    await everyTrackSounds(page, tracks, 'the arrangement')
    await mcp.call('transport_stop')

    // What the agent made, read back through its own tools.
    const listing = (await mcp.call<{ tracks: { id: string; kind: string; clip_count: number }[] }>('track_list')).tracks
    expect(listing.map((t) => t.id)).toEqual(tracks)
    expect(listing.map((t) => t.kind)).toEqual(['midi', 'midi', 'midi', 'audio'])
    const scened = (await mcp.call<{ scenes: { clips: unknown[] }[] }>('scene_list')).scenes
    expect(scened.slice(0, 2).map((s) => s.clips.length)).toEqual([4, 4])
    expect((await mcp.call<{ points: unknown[] }>('automation_lane_points', { lane })).points).toHaveLength(2)
    for (const track of tracks) expect((await mcp.call<{ placement_count: number }>('track_get', { track })).placement_count).toBeGreaterThan(0)

    // The history names the agent as the author of its edits.
    await page.getByTestId('panel-history').click()
    await expect(page.locator(`[data-testid=history-entry][data-author="agent:${CLIENT}"]`).first()).toBeVisible()
    await expect(page.locator('[data-testid=history-entry][data-author=user]').first()).toBeVisible() // the import, the user's

    // The user exports the song: not silent, and the output protection silenced nothing (no NaN, no infinity).
    await page.getByTestId('project-menu').click()
    await page.getByTestId('menu-export').click()
    const dialog = page.getByTestId('export-dialog')
    await page.getByTestId('export-start').click()
    await expect(dialog).toHaveAttribute('data-phase', 'done', { timeout: 60_000 })
    await expect(dialog).toHaveAttribute('data-silenced-frames', '0')
    const name = await page.evaluate(() => (window as unknown as { __picks: string[] }).__picks[0]!)
    const wav = await readWav(page, name)
    expect(wav.bits).toBe(24)
    expect(wav.frames).toBeGreaterThanOrEqual(2 * 48_000)
    console.log(
      `export: ${(wav.frames / 48_000).toFixed(2)} s, peak ${(20 * Math.log10(wav.peak)).toFixed(1)} dBFS, frames silenced for a non-finite sample: 0`,
    )
    expect(wav.peak).toBeGreaterThan(0.05)
    expect(wav.peak).toBeLessThanOrEqual(1)
  } finally {
    mcp.close()
  }
  expect(errors).toEqual([])
})
