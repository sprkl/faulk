// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import { NativeEngine } from '../engine/native-engine'
import type { EngineEvents } from '../engine/engine'
import { SINGLE_SYNTH, type Command } from '../generated/protocol'
import { DEFAULT_PARAMS } from '../params'
import { CoreError, ProjectCore, type FolderEntry } from '../project/core'
import { readFolder, SONG_FOLDER } from '../project/reference-folder'
import { REFUSAL_TOAST_MS } from './dispatcher'
import { TOKEN_KEY } from './storage'

const pairHelper = vi.fn()
type Probe = { status: 'absent' } | { status: 'available' }
const probeHelper = vi.fn<() => Promise<Probe>>(() => Promise.resolve({ status: 'available' }))
vi.mock('../engine/helper-client', () => ({
  pairHelper: (code: string) => pairHelper(code),
  probeHelper: () => probeHelper(),
}))
interface FakeWeb {
  kind: 'web'
  send: ReturnType<typeof vi.fn>
  push: ReturnType<typeof vi.fn>
  status: () => object
  suspend: Mock<() => Promise<void>>
  resume: Mock<() => Promise<void>>
  close: ReturnType<typeof vi.fn>
}
let webEvents: EngineEvents | null = null
let webs: FakeWeb[] = []
vi.mock('../engine/web-engine', () => ({
  WebEngine: {
    create: (events: EngineEvents) => {
      webEvents = events
      const web: FakeWeb = {
        kind: 'web',
        send: vi.fn(),
        push: vi.fn(),
        status: () => ({ sampleRate: 48000, bufferFrames: 128, estimatedLatencyMs: null }),
        suspend: vi.fn(() => Promise.resolve()),
        resume: vi.fn(() => Promise.resolve()),
        close: vi.fn(() => Promise.resolve()),
      }
      webs.push(web)
      return Promise.resolve(web)
    },
  },
}))
// The project core runs for real (the module built from crates/project-wasm).
vi.mock('../project/core', async (original) => ({
  ...(await original<typeof import('../project/core')>()),
  coreModule: () => import('node:fs').then((fs) => WebAssembly.compile(fs.readFileSync(`${process.cwd()}/src/lib/project/wasm/project.wasm`))),
}))
let nativeEvents: EngineEvents | null = null
let nativeEngine: { kind: 'native'; status: object; send: ReturnType<typeof vi.fn>; close: ReturnType<typeof vi.fn> } | null = null
vi.mock('../engine/native-engine', () => ({
  NativeEngine: {
    connect: (_token: string, events: EngineEvents) => {
      nativeEvents = events
      return nativeEngine ? Promise.resolve(nativeEngine) : Promise.reject(new Error('no helper in tests'))
    },
  },
}))
// Web MIDI is off, except in the tests that turn it on: then `releaseAll` is what the test controls.
let midiOn = false
let midiReleaseAll: () => Promise<void> = () => Promise.resolve()
vi.mock('../midi/web-midi', () => ({
  WebMidiHost: class {
    static supported = () => midiOn
    enable = () => Promise.resolve(true)
    releaseAll = () => midiReleaseAll()
    select = vi.fn()
    refresh = vi.fn()
  },
}))

const { createStores } = await import('./index')

const newSession = () => createStores('arrangement', 'devices')

function address(hash: string) {
  const location = { hash, pathname: '/app', search: '' } as Location
  const clear = vi.fn<(url: string) => void>()
  return { location, clear }
}

describe('pairing link in the session', () => {
  beforeEach(() => {
    pairHelper.mockReset()
    pairHelper.mockResolvedValue({ status: 'refused', attemptsLeft: 4 })
  })

  it('keeps the code until audio starts, then pairs once', async () => {
    const session = newSession()
    const { location, clear } = address('#pair=12345678')
    session.dispatcher.takePairLink(location, clear)
    expect(clear).toHaveBeenCalledWith('/app')
    expect(session.engine.pendingPairCode).toBe('12345678')
    expect(pairHelper).not.toHaveBeenCalled()

    await session.dispatcher.start()
    expect(pairHelper).toHaveBeenCalledTimes(1)
    expect(pairHelper).toHaveBeenCalledWith('12345678')
    expect(session.engine.pendingPairCode).toBeNull()
  })

  it('pairs at once when audio already runs', async () => {
    const session = newSession()
    await session.dispatcher.start()
    const { location, clear } = address('#pair=87654321')
    session.dispatcher.takePairLink(location, clear)
    await vi.waitFor(() => expect(pairHelper).toHaveBeenCalledWith('87654321'))
    expect(session.engine.pendingPairCode).toBeNull()
  })

  it('reports a pairing failure as such, not as an audio start failure', async () => {
    pairHelper.mockResolvedValue({ status: 'failed' })
    const session = newSession()
    const { location, clear } = address('#pair=12345678')
    session.dispatcher.takePairLink(location, clear)
    await session.dispatcher.start()
    expect(session.engine.started).toBe(true)
    expect(session.engine.notice).toBeNull()
    expect(session.engine.pairingMessage).toEqual({ key: 'helper.failed' })
  })

  it('keeps the audio started when pairing throws unexpectedly', async () => {
    pairHelper.mockRejectedValue(new Error('boom'))
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const session = newSession()
    const { location, clear } = address('#pair=12345678')
    session.dispatcher.takePairLink(location, clear)
    await session.dispatcher.start()
    expect(session.engine.started).toBe(true)
    expect(session.engine.notice).toBeNull()
    expect(session.engine.pairingMessage).toEqual({ key: 'helper.failed' })
  })

  it('reports a pairing failure on a link that arrives after the start', async () => {
    pairHelper.mockRejectedValue(new Error('boom'))
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const session = newSession()
    await session.dispatcher.start()
    const { location, clear } = address('#pair=87654321')
    session.dispatcher.takePairLink(location, clear)
    await vi.waitFor(() => expect(session.engine.pairingMessage).toEqual({ key: 'helper.failed' }))
    expect(session.engine.started).toBe(true)
  })

  it('ignores an invalid or missing link', async () => {
    const session = newSession()
    session.dispatcher.takePairLink(...(Object.values(address('#pair=12')) as [Location, (url: string) => void]))
    session.dispatcher.takePairLink(...(Object.values(address('')) as [Location, (url: string) => void]))
    expect(session.engine.pendingPairCode).toBeNull()
    await session.dispatcher.start()
    expect(pairHelper).not.toHaveBeenCalled()
  })
})

describe('probing again once the browser allows local access', () => {
  beforeEach(() => {
    probeHelper.mockReset()
    probeHelper.mockResolvedValue({ status: 'available' })
  })

  it('probes again when the last probe found nothing', async () => {
    const session = newSession()
    probeHelper.mockResolvedValueOnce({ status: 'absent' })
    await session.dispatcher.checkHelper()
    expect(session.engine.helper).toBe('absent')
    void session.dispatcher.onLocalAccessGranted()
    await vi.waitFor(() => expect(session.engine.helper).toBe('available'))
    expect(probeHelper).toHaveBeenCalledTimes(2)
  })

  it('lets the newer probe decide when the probe behind the prompt gives up later', async () => {
    const session = newSession()
    let giveUp: (p: Probe) => void = () => {}
    probeHelper.mockReturnValueOnce(new Promise<Probe>((r) => (giveUp = r)))
    const first = session.dispatcher.checkHelper()
    expect(session.engine.helper).toBe('checking')
    void session.dispatcher.onLocalAccessGranted()
    await vi.waitFor(() => expect(session.engine.helper).toBe('available'))
    giveUp({ status: 'absent' })
    await first
    expect(session.engine.helper).toBe('available')
  })

  it('returns what an older probe found even when a newer probe decides the state', async () => {
    const session = newSession()
    let answer: (p: Probe) => void = () => {}
    probeHelper.mockReturnValueOnce(new Promise<Probe>((r) => (answer = r)))
    probeHelper.mockReturnValueOnce(new Promise<Probe>(() => {}))
    const older = session.dispatcher.checkHelper()
    void session.dispatcher.onLocalAccessGranted()
    answer({ status: 'available' })
    expect(await older).toBe('available')
    expect(session.engine.helper).toBe('checking')
  })

  it('does not probe when the helper is already found', async () => {
    const session = newSession()
    await session.dispatcher.checkHelper()
    expect(session.engine.helper).toBe('available')
    void session.dispatcher.onLocalAccessGranted()
    expect(probeHelper).toHaveBeenCalledTimes(1)
  })
})

describe('a pairing link that could not reach the helper behind the prompt', () => {
  beforeEach(() => {
    probeHelper.mockReset()
    probeHelper.mockResolvedValue({ status: 'available' })
    pairHelper.mockReset()
    localStorage.clear()
  })

  async function startedWithFailedLink() {
    probeHelper.mockResolvedValueOnce({ status: 'absent' })
    pairHelper.mockResolvedValueOnce({ status: 'failed' })
    const session = newSession()
    await session.dispatcher.checkHelper()
    const { location, clear } = address('#pair=12345678')
    session.dispatcher.takePairLink(location, clear)
    await session.dispatcher.start()
    expect(session.engine.pairingMessage).toEqual({ key: 'helper.failed' })
    return session
  }

  it('pairs again with the same code once access is granted and the helper is found', async () => {
    const session = await startedWithFailedLink()
    pairHelper.mockResolvedValueOnce({ status: 'refused', attemptsLeft: 4 })
    await session.dispatcher.onLocalAccessGranted()
    expect(pairHelper).toHaveBeenCalledTimes(2)
    expect(pairHelper).toHaveBeenLastCalledWith('12345678')
  })

  it('tries the code only once more', async () => {
    const session = await startedWithFailedLink()
    pairHelper.mockResolvedValueOnce({ status: 'refused', attemptsLeft: 4 })
    await session.dispatcher.onLocalAccessGranted()
    session.engine.helper = 'absent'
    await session.dispatcher.onLocalAccessGranted()
    expect(pairHelper).toHaveBeenCalledTimes(2)
  })

  it('does not retry a code the helper refused', async () => {
    probeHelper.mockResolvedValueOnce({ status: 'absent' })
    pairHelper.mockResolvedValueOnce({ status: 'refused', attemptsLeft: 4 })
    const session = newSession()
    await session.dispatcher.checkHelper()
    const { location, clear } = address('#pair=12345678')
    session.dispatcher.takePairLink(location, clear)
    await session.dispatcher.start()
    await session.dispatcher.onLocalAccessGranted()
    expect(pairHelper).toHaveBeenCalledTimes(1)
  })

  it('does not retry while the helper is still out of reach', async () => {
    const session = await startedWithFailedLink()
    probeHelper.mockResolvedValueOnce({ status: 'absent' })
    await session.dispatcher.onLocalAccessGranted()
    expect(pairHelper).toHaveBeenCalledTimes(1)
  })
})

const SYNTH_TRACK = '00000000000000000000000001'
const SYNTH_DEVICE = '00000000000000000000000002'
const setParamCommand = (param: string, value: number) => ({ type: 'device.set_param', device: SYNTH_DEVICE, param, value }) as Command
const noteOn = (note: number) => ({ type: 'live.note_on', track: SYNTH_TRACK, note, velocity: 100 }) as Command
const firstParam = Object.keys(DEFAULT_PARAMS)[0] as keyof typeof DEFAULT_PARAMS

describe('running commands through the project core', () => {
  beforeEach(() => {
    webs = []
    nativeEngine = null
    midiOn = false
    midiReleaseAll = () => Promise.resolve()
    localStorage.clear()
  })

  it('does nothing before audio starts', () => {
    const session = newSession()
    session.dispatcher.run(noteOn(60))
    expect(session.engine.activeNotes).toEqual([])
  })

  it('sets the engine up from the project and feeds the store from events', async () => {
    const session = newSession()
    await session.dispatcher.start()
    expect(webs[0].push).toHaveBeenCalledTimes(1)
    expect(webs[0].push.mock.calls[0][0].length).toBeGreaterThan(0)
    session.dispatcher.run(setParamCommand(firstParam, 0.123))
    expect(session.project.params[firstParam]).toBe(0.123)
    expect(session.project.canUndo).toBe(true)
    const [command, bytes] = webs[0].send.mock.calls.at(-1)!
    expect(command.type).toBe('device.set_param')
    expect(bytes.length).toBeGreaterThan(0)
  })

  it('shows the slots the engine holds the tracks in, again after each snapshot (the launch grid reads the engine by them)', async () => {
    const session = newSession()
    await session.dispatcher.start()
    expect(session.project.view?.slots).toEqual({ epoch: 1, tracks: { [SINGLE_SYNTH.track]: 0 }, scenes: {} })
    webEvents?.onStreamRefused?.(-3, 128)
    await session.dispatcher.start()
    expect(session.project.view?.slots.epoch).toBe(2)
    expect(session.project.view?.slots.tracks).toEqual({ [SINGLE_SYNTH.track]: 0 })
  })

  it('sends a played note as engine bytes and shows it sounding', async () => {
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run(noteOn(60))
    expect(webs[0].send.mock.calls.at(-1)![1].length).toBeGreaterThan(0)
    expect(session.engine.activeNotes).toEqual([60])
  })

  it('sends nothing for a command the project refuses', async () => {
    const session = newSession()
    await session.dispatcher.start()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const before = webs[0].send.mock.calls.length
    session.dispatcher.run({ type: 'device.set_param', device: '00000000000000000000000009', param: firstParam, value: 1 } as Command)
    expect(webs[0].send.mock.calls.length).toBe(before)
    expect(session.project.params[firstParam]).toBe(DEFAULT_PARAMS[firstParam])
  })

  describe('a command the core refuses', () => {
    const missing = { type: 'track.rename', track: '00000000000000000000000009', name: 'x' } as Command
    const quiet = () => vi.spyOn(console, 'warn').mockImplementation(() => {})

    it('is shown to the user by its code, and nothing else changes', async () => {
      const warn = quiet()
      const session = newSession()
      await session.dispatcher.start()
      const before = webs[0].send.mock.calls.length
      expect(session.engine.refusal).toBeNull()
      session.dispatcher.run(missing)
      expect(session.engine.refusal).toMatchObject({ key: 'refusal.not_found', count: 1 })
      expect(webs[0].send.mock.calls.length).toBe(before)
      warn.mockRestore()
    })

    it('replaces the toast, counts the same refusal, and goes away after a while', async () => {
      const warn = quiet()
      const session = newSession()
      await session.dispatcher.start()
      vi.useFakeTimers()
      try {
        session.dispatcher.run(missing)
        session.dispatcher.run(missing)
        session.dispatcher.run(missing)
        expect(session.engine.refusal).toMatchObject({ key: 'refusal.not_found', count: 3 })
        // A malformed command is a page bug, not a refusal of the core: the toast is left as it is.
        session.dispatcher.run({ type: 'nope' } as unknown as Command)
        expect(session.engine.refusal).toMatchObject({ key: 'refusal.not_found', count: 3 })
        vi.advanceTimersByTime(REFUSAL_TOAST_MS - 1)
        expect(session.engine.refusal).not.toBeNull()
        // A new refusal restarts the delay.
        session.dispatcher.run(missing)
        vi.advanceTimersByTime(REFUSAL_TOAST_MS - 1)
        expect(session.engine.refusal).toMatchObject({ count: 4 })
        vi.advanceTimersByTime(1)
        expect(session.engine.refusal).toBeNull()
        // After the delay the count starts over.
        session.dispatcher.run(missing)
        expect(session.engine.refusal).toMatchObject({ count: 1 })
      } finally {
        vi.useRealTimers()
      }
      warn.mockRestore()
    })

    it('shows nothing for a caller that reports the refusal itself (quiet)', async () => {
      const warn = quiet()
      const session = newSession()
      await session.dispatcher.start()
      session.dispatcher.run({ type: 'history.begin_group' } as Command)
      // An undo during an open group is refused; shown unless the caller asked for quiet.
      session.dispatcher.run({ type: 'history.undo' } as Command, { quiet: true })
      expect(session.engine.refusal).toBeNull()
      session.dispatcher.run({ type: 'history.undo' } as Command)
      expect(session.engine.refusal).toMatchObject({ key: 'refusal.invalid' })
      warn.mockRestore()
    })

    it('shows nothing for a command that applies, and nothing for a malformed one', async () => {
      const warn = quiet()
      const session = newSession()
      await session.dispatcher.start()
      session.dispatcher.run(setParamCommand(firstParam, 0.4))
      session.dispatcher.run({ type: 'nope' } as unknown as Command)
      expect(session.engine.refusal).toBeNull()
      warn.mockRestore()
    })
  })

  it('stops the engine on a refused stream, shows the error, and restarts from a snapshot that keeps the project', async () => {
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run(setParamCommand(firstParam, 0.321))
    webEvents?.onStreamRefused?.(-3, 128)
    expect(session.engine.streamError).toEqual({ code: -3, at: 128 })
    expect(session.engine.started).toBe(false)
    expect(webs[0].close).toHaveBeenCalledTimes(1)
    // A second report from the same engine changes nothing and does not close it again.
    webEvents?.onStreamRefused?.(-3, 129)
    expect(webs[0].close).toHaveBeenCalledTimes(1)
    // Commands with no engine are not applied.
    session.dispatcher.run(setParamCommand(firstParam, 0.9))
    expect(session.project.params[firstParam]).toBe(0.321)

    await session.dispatcher.start()
    expect(session.engine.streamError).toBeNull()
    expect(session.engine.started).toBe(true)
    expect(webs).toHaveLength(2)
    expect(webs[1].push).toHaveBeenCalledTimes(1)
    expect(session.project.params[firstParam]).toBe(0.321)
  })

  it('ignores a refusal reported late by the engine that was stopped', async () => {
    const session = newSession()
    await session.dispatcher.start()
    const oldEvents = webEvents!
    oldEvents.onStreamRefused?.(-3, 0)
    await session.dispatcher.start()
    oldEvents.onStreamRefused?.(-3, 0)
    expect(session.engine.streamError).toBeNull()
    expect(session.engine.started).toBe(true)
    expect(webs[1].close).not.toHaveBeenCalled()
  })

  it('sends the all-sound-off command to the engine as it is (the double-click on Stop)', async () => {
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run({ type: 'live.all_sound_off' })
    expect(webs[0].send.mock.calls.at(-1)![0]).toEqual({ type: 'live.all_sound_off' })
  })

  it('keeps the helper in step in Boost mode and catches the Web engine up by snapshot on return', async () => {
    nativeEngine = {
      kind: 'native',
      status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
      send: vi.fn(),
      close: vi.fn(() => Promise.resolve()),
    }
    localStorage.setItem(TOKEN_KEY, 'x')
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run(setParamCommand(firstParam, 0.4))
    await session.dispatcher.enterBoost()
    expect(session.engine.mode).toBe('native')
    // The helper's copy is brought to the page's settings, by commands.
    expect(nativeEngine.send.mock.calls.some(([c]) => c.type === 'device.set_param' && c.value === 0.4)).toBe(true)
    // Held notes are released on the Web side before it is suspended.
    expect(webs[0].send.mock.calls.at(-1)![0].type).toBe('live.all_notes_off')
    const pushes = webs[0].push.mock.calls.length
    session.dispatcher.run(setParamCommand(firstParam, 0.5))
    expect(nativeEngine.send.mock.calls.at(-1)![0]).toMatchObject({ type: 'device.set_param', value: 0.5 })
    expect(webs[0].push.mock.calls.length).toBe(pushes)
    expect(webs[0].send.mock.calls.at(-1)![0].type).not.toBe('device.set_param')

    await session.dispatcher.enterWeb()
    expect(webs[0].push.mock.calls.length).toBe(pushes + 1)
    expect(session.engine.mode).toBe('web')
    expect(session.project.params[firstParam]).toBe(0.5)
  })

  describe.each([
    ['the button', false],
    ['the helper being lost', true],
  ])('a note played while the page goes back to Web, by %s (#262)', (_, lost) => {
    it('reaches the Web engine after its snapshot, which would otherwise wipe it', async () => {
      let finishClose: () => void = () => {}
      let finishResume: () => void = () => {}
      nativeEngine = {
        kind: 'native',
        status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
        send: vi.fn(),
        close: vi.fn(() => new Promise<void>((resolve) => (finishClose = resolve))),
      }
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      await session.dispatcher.enterBoost()
      webs[0].resume.mockImplementation(() => new Promise<void>((resolve) => (finishResume = resolve)))
      const pushesBefore = webs[0].push.mock.calls.length
      if (lost) nativeEvents!.onClosed?.()
      const back = lost ? Promise.resolve() : session.dispatcher.enterWeb()
      // The key goes down while the switch is under way: the audio context is still resuming.
      session.dispatcher.run(noteOn(60))
      expect(session.engine.mode).toBe('web')
      expect(nativeEngine.send.mock.calls.some(([c]) => c.type === 'live.note_on')).toBe(false)
      const snapshot = webs[0].push.mock.invocationCallOrder[pushesBefore]
      const note = webs[0].send.mock.invocationCallOrder.at(-1)!
      expect(snapshot).toBeDefined()
      expect(note).toBeGreaterThan(snapshot)
      expect(webs[0].send.mock.calls.at(-1)![0].type).toBe('live.note_on')
      // The switch still finishes: the helper closes, the audio context resumes.
      finishClose()
      await vi.waitFor(() => expect(webs[0].resume).toHaveBeenCalled())
      finishResume()
      await back
      expect(webs[0].push.mock.calls.length).toBe(pushesBefore + 1)
    })
  })

  it('takes a new snapshot when the helper is lost after Boost was entered again during the switch', async () => {
    const resumes: (() => void)[] = []
    nativeEngine = {
      kind: 'native',
      status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
      send: vi.fn(),
      close: vi.fn(() => Promise.resolve()),
    }
    localStorage.setItem(TOKEN_KEY, 'x')
    const session = newSession()
    await session.dispatcher.start()
    await session.dispatcher.enterBoost()
    webs[0].resume.mockImplementation(() => new Promise<void>((resolve) => resumes.push(resolve)))
    const first = session.dispatcher.enterWeb()
    await session.dispatcher.enterBoost()
    expect(session.engine.mode).toBe('native')
    const pushes = webs[0].push.mock.calls.length
    nativeEvents!.onClosed?.()
    expect(webs[0].push.mock.calls.length).toBe(pushes + 1)
    expect(session.engine.mode).toBe('web')
    resumes.forEach((resume) => resume())
    await first
  })

  it('joins a switch to Web already under way instead of taking a second snapshot', async () => {
    nativeEngine = {
      kind: 'native',
      status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
      send: vi.fn(),
      close: vi.fn(() => Promise.resolve()),
    }
    localStorage.setItem(TOKEN_KEY, 'x')
    const session = newSession()
    await session.dispatcher.start()
    await session.dispatcher.enterBoost()
    const pushesBefore = webs[0].push.mock.calls.length
    await Promise.all([session.dispatcher.enterWeb(), session.dispatcher.enterWeb()])
    expect(webs[0].push.mock.calls.length).toBe(pushesBefore + 1)
  })

  describe('Boost failure paths (#272)', () => {
    const boostEngine = () =>
      (nativeEngine = {
        kind: 'native',
        status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
        send: vi.fn(),
        close: vi.fn(() => Promise.resolve()),
      })

    it('leaves the Web engine alone when the helper cannot be reached: a held note is neither released nor re-sent as a snapshot', async () => {
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      session.dispatcher.run(noteOn(60))
      const pushes = webs[0].push.mock.calls.length
      const sends = webs[0].send.mock.calls.length
      await session.dispatcher.enterBoost()
      expect(webs[0].push.mock.calls.length).toBe(pushes)
      expect(webs[0].send.mock.calls.length).toBe(sends)
      expect(webs[0].suspend).not.toHaveBeenCalled()
      expect(session.engine.mode).toBe('web')
      expect(session.engine.activeNotes).toEqual([60])
    })

    it('opens one helper session when Boost is asked twice while connecting', async () => {
      const native = boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      const connect = vi.spyOn(NativeEngine, 'connect')
      await Promise.all([session.dispatcher.enterBoost(), session.dispatcher.enterBoost()])
      expect(connect).toHaveBeenCalledTimes(1)
      expect(native.close).not.toHaveBeenCalled()
      expect(session.engine.mode).toBe('native')
    })

    it('closes a helper session that never became current when releasing the Web side fails', async () => {
      const native = boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      webs[0].send.mockImplementation(() => {
        throw new Error('trap')
      })
      await session.dispatcher.enterBoost()
      expect(native.close).toHaveBeenCalledTimes(1)
      expect(session.engine.mode).toBe('web')
      expect(localStorage.getItem(TOKEN_KEY)).toBe('x')
    })

    it('connects with the token of a pairing that happened while a refused connection was under way', async () => {
      boostEngine()
      localStorage.setItem(TOKEN_KEY, 'old')
      const session = newSession()
      await session.dispatcher.start()
      let refuse: (error: Error) => void = () => {}
      const connect = vi.spyOn(NativeEngine, 'connect').mockImplementationOnce(() => new Promise((_, reject) => (refuse = reject)))
      const stale = session.dispatcher.enterBoost()
      // The pairing stores the new token and asks for Boost: it waits for the refused attempt.
      localStorage.setItem(TOKEN_KEY, 'new')
      const paired = session.dispatcher.enterBoost()
      refuse(new Error('refused'))
      await Promise.all([stale, paired])
      expect(connect.mock.calls.map(([token]) => token)).toEqual(['old', 'new'])
      expect(localStorage.getItem(TOKEN_KEY)).toBe('new')
      expect(session.engine.mode).toBe('native')
    })

    it('does not suspend the Web engine when the page went back to Web while the MIDI inputs were closing', async () => {
      midiOn = true
      boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      let finishRelease: () => void = () => {}
      midiReleaseAll = () => new Promise<void>((resolve) => (finishRelease = resolve))
      const boost = session.dispatcher.enterBoost()
      await vi.waitFor(() => expect(session.engine.mode).toBe('native'))
      const back = session.dispatcher.enterWeb()
      finishRelease()
      await Promise.all([boost, back])
      expect(session.engine.mode).toBe('web')
      expect(webs[0].suspend).not.toHaveBeenCalled()
    })

    it('ignores the MIDI activity of a helper session that is already closed', async () => {
      boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      await session.dispatcher.enterBoost()
      const closed = nativeEvents!
      await session.dispatcher.enterWeb()
      closed.onMidiActivity?.(60, 100)
      expect(session.engine.activeNotes).toEqual([])
    })

    it('does not pass the failure of another attempt on to a Boost request that waited for it', async () => {
      boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      webs[0].suspend.mockImplementationOnce(() => Promise.reject(new Error('suspend failed')))
      webs[0].resume.mockImplementationOnce(() => Promise.reject(new Error('resume failed')))
      const first = session.dispatcher.enterBoost()
      const second = session.dispatcher.enterBoost()
      await expect(first).rejects.toThrow('resume failed')
      await second
      expect(session.engine.mode).toBe('native')
    })

    it('keeps the pairing when the helper is lost while the Web engine suspends and that step then fails', async () => {
      boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      webs[0].suspend.mockImplementation(() => {
        nativeEvents!.onClosed?.()
        return Promise.reject(new Error('suspend failed'))
      })
      await session.dispatcher.enterBoost()
      expect(session.engine.mode).toBe('web')
      expect(localStorage.getItem(TOKEN_KEY)).toBe('x')
    })

    it('closes the helper session when a step after the connection fails', async () => {
      const native = boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      webs[0].suspend.mockImplementation(() => Promise.reject(new Error('suspend failed')))
      await session.dispatcher.enterBoost()
      expect(native.close).toHaveBeenCalledTimes(1)
      expect(session.engine.mode).toBe('web')
      // The helper accepted the session: the pairing stays.
      expect(localStorage.getItem(TOKEN_KEY)).toBe('x')
      // The page plays again in the Web engine.
      session.dispatcher.run(noteOn(60))
      expect(native.send.mock.calls.some(([c]) => c.type === 'live.note_on')).toBe(false)
      expect(webs[0].send.mock.calls.at(-1)![0].type).toBe('live.note_on')
    })

    it('ignores an audio.stream error of a helper session that is already closed', async () => {
      boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      await session.dispatcher.enterBoost()
      const closed = nativeEvents!
      await session.dispatcher.enterWeb()
      const pushes = webs[0].push.mock.calls.length
      closed.onError?.('audio.stream')
      expect(webs[0].push.mock.calls.length).toBe(pushes)
      expect(session.engine.notice).toBeNull()
    })

    it('ignores a message of an older helper session while a newer one is current', async () => {
      boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      await session.dispatcher.enterBoost()
      const older = nativeEvents!
      await session.dispatcher.enterWeb()
      await session.dispatcher.enterBoost()
      expect(session.engine.mode).toBe('native')
      const pushes = webs[0].push.mock.calls.length
      older.onError?.('audio.stream')
      older.onClosed?.()
      expect(session.engine.mode).toBe('native')
      expect(webs[0].push.mock.calls.length).toBe(pushes)
    })

    it('still goes back to Web on an audio.stream error of the current helper session', async () => {
      boostEngine()
      localStorage.setItem(TOKEN_KEY, 'x')
      const session = newSession()
      await session.dispatcher.start()
      await session.dispatcher.enterBoost()
      const pushes = webs[0].push.mock.calls.length
      nativeEvents!.onError?.('audio.stream')
      expect(session.engine.mode).toBe('web')
      expect(webs[0].push.mock.calls.length).toBe(pushes + 1)
    })
  })

  it('stops the engine when the core cannot encode the command it applied', async () => {
    const session = newSession()
    await session.dispatcher.start()
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const run = vi.spyOn(ProjectCore.prototype, 'run').mockImplementation(() => ({
      report: { status: 'applied', output: { type: 'live.note_on' }, events: [], stream_error: -4 } as never,
      engine: new Uint8Array(),
    }))
    session.dispatcher.run(noteOn(60))
    run.mockRestore()
    expect(session.engine.streamError).toEqual({ code: -4, at: 0 })
    expect(session.engine.started).toBe(false)
    expect(webs[0].close).toHaveBeenCalledTimes(1)
  })

  it('still sends the helper the command the core could not encode, in Boost mode', async () => {
    nativeEngine = {
      kind: 'native',
      status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
      send: vi.fn(),
      close: vi.fn(() => Promise.resolve()),
    }
    localStorage.setItem(TOKEN_KEY, 'x')
    const session = newSession()
    await session.dispatcher.start()
    await session.dispatcher.enterBoost()
    const run = vi.spyOn(ProjectCore.prototype, 'run').mockImplementation(() => ({
      report: { status: 'applied', output: { type: 'live.note_on' }, events: [], stream_error: -4 } as never,
      engine: new Uint8Array(),
    }))
    const command = noteOn(61)
    session.dispatcher.run(command)
    run.mockRestore()
    expect(nativeEngine.send).toHaveBeenLastCalledWith(command)
    expect(session.engine.streamError).toBeNull()
    expect(webs[0].close).not.toHaveBeenCalled()
  })

  it('reads the project again when the report of an applied command was lost', async () => {
    const session = newSession()
    await session.dispatcher.start()
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const spy = vi.spyOn(ProjectCore.prototype, 'run').mockImplementation(() => {
      throw new CoreError('run', -3)
    })
    const project = vi.spyOn(ProjectCore.prototype, 'project')
    session.dispatcher.run(setParamCommand(firstParam, 0.2))
    spy.mockRestore()
    expect(project).toHaveBeenCalled()
    expect(session.engine.streamError).toEqual({ code: -3, at: 0 })
  })

  it('stops the engine when the core cannot give a snapshot', async () => {
    const session = newSession()
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const setup = vi.spyOn(ProjectCore.prototype, 'setup').mockImplementation(() => {
      throw new CoreError('setup', -3)
    })
    await session.dispatcher.start()
    setup.mockRestore()
    expect(session.engine.streamError).toEqual({ code: -3, at: 0 })
    expect(session.engine.started).toBe(false)
    expect(webs[0].close).toHaveBeenCalledTimes(1)
    expect(session.engine.notice).toBeNull()
  })

  it('names no command in the dispatcher', async () => {
    const { readFileSync } = await import('node:fs')
    const source = readFileSync(`${process.cwd()}/src/lib/stores/dispatcher.ts`, 'utf8')
    expect(source).not.toMatch(
      /command\.type|type: '|'(project|track|device|clip|note|scene|launch|arrangement|automation|transport|mixer|record|midi(?!.connect)|live|history|view)\.[a-z_.]+'/,
    )
    expect(source).not.toMatch(/from '\.\.\/synth-commands'/)
  })
})

describe('opening a project folder (SV-4)', () => {
  const SONG_TRACK = '01HF000000000000000000000J'
  const song = () => readFolder(SONG_FOLDER)
  const withFile = (entries: FolderEntry[], path: string, text: string): FolderEntry[] => [
    ...entries.filter((e) => e.path !== path),
    { path, bytes: new TextEncoder().encode(text) },
  ]
  const boostEngine = () =>
    (nativeEngine = {
      kind: 'native',
      status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
      send: vi.fn(),
      close: vi.fn(() => Promise.resolve()),
    })

  beforeEach(() => {
    webs = []
    nativeEngine = null
    midiOn = false
    midiReleaseAll = () => Promise.resolve()
    localStorage.clear()
  })

  it('replaces the project in the store, the history with it, and starts the engine over from a snapshot', async () => {
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run(setParamCommand(firstParam, 0.3))
    session.dispatcher.run(noteOn(60))
    expect(session.project.canUndo).toBe(true)
    expect(session.engine.activeNotes).toEqual([60])
    const pushes = webs[0].push.mock.calls.length
    const epoch = session.project.view!.slots.epoch

    expect(await session.dispatcher.openProject('song', song())).toEqual({ status: 'opened', name: 'Song "one"' })
    expect(session.project.view?.project.name).toBe('Song "one"')
    expect(Object.keys(session.project.view!.project.tracks)).toHaveLength(2)
    expect(session.project.canUndo).toBe(false)
    expect(session.project.canRedo).toBe(false)
    expect(session.engine.activeNotes).toEqual([])
    expect(session.engine.notice).toBeNull()
    // The engine started over from the snapshot of the new project: the old voices end with the old state.
    expect(webs[0].push.mock.calls.length).toBe(pushes + 1)
    expect(webs[0].push.mock.calls.at(-1)![0].length).toBeGreaterThan(0)
    expect(session.project.view!.slots.epoch).toBe(epoch + 1)
    expect(Object.keys(session.project.view!.slots.tracks)).toHaveLength(2)
    expect(session.engine.started).toBe(true)
  })

  it('restores the session of the folder: solo, selection and the MIDI input', async () => {
    midiOn = true
    const session = newSession()
    await session.dispatcher.start()
    const midi = (session.dispatcher as unknown as { midi: { select: ReturnType<typeof vi.fn> } }).midi
    const sessionFile = JSON.stringify({ selected_track: SONG_TRACK, solo: [SONG_TRACK], midi_input: 'in-1' })
    await session.dispatcher.openProject('song', withFile(song(), '.faulk/session.json', sessionFile))
    expect(session.project.soloed).toEqual([SONG_TRACK])
    expect(session.project.selected?.id).toBe(SONG_TRACK)
    expect(midi.select).toHaveBeenLastCalledWith('in-1')
    // Another session without an input: the page closes the previous project's input.
    await session.dispatcher.openProject('song', song())
    expect(session.project.soloed).toEqual([])
    expect(midi.select).toHaveBeenLastCalledWith(null)
  })

  it('opens before audio starts: the first engine then starts from the opened project', async () => {
    const session = newSession()
    expect(await session.dispatcher.openProject('song', song())).toEqual({ status: 'opened', name: 'Song "one"' })
    expect(session.project.view?.project.name).toBe('Song "one"')
    expect(session.engine.started).toBe(false)
    await session.dispatcher.start()
    expect(webs[0].push).toHaveBeenCalledTimes(1)
    expect(webs[0].push.mock.calls[0][0].length).toBeGreaterThan(0)
    expect(session.project.view?.project.name).toBe('Song "one"')
    expect(Object.keys(session.project.view!.slots.tracks)).toHaveLength(2)
  })

  it('refuses what it cannot read with a notice and changes nothing', async () => {
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run(setParamCommand(firstParam, 0.3))
    const view = session.project.view
    const pushes = webs[0].push.mock.calls.length
    const sends = webs[0].send.mock.calls.length
    const newer = withFile(
      song(),
      'project.json',
      new TextDecoder().decode((song().find((e) => e.path === 'project.json') as { bytes: Uint8Array }).bytes).replace('"format": 1', '"format": 99'),
    )
    const cases: [string, FolderEntry[], string, string][] = [
      ['a newer format', newer, 'newer_format', 'project.open.newer'],
      ['no project', [{ path: 'x.txt', bytes: new Uint8Array([1]) }], 'not_a_project', 'project.open.notAProject'],
      ['a damaged file', withFile(song(), 'project.json', '{"format": 1'), 'corrupt', 'project.open.invalid'],
      ['entries that are not a folder', [{ path: '../x', bytes: new Uint8Array() }], 'frame', 'project.open.invalid'],
    ]
    for (const [what, entries, reason, notice] of cases) {
      session.engine.notice = null
      expect(await session.dispatcher.openProject('p', entries), what).toEqual({ status: 'refused', reason })
      expect(session.engine.notice, what).toBe(notice)
      expect(session.project.view, what).toBe(view)
      expect(session.project.canUndo, what).toBe(true)
    }
    expect(webs[0].push.mock.calls.length).toBe(pushes)
    // Nothing was sent to the engine: a refusal leaves the page as it was.
    expect(webs[0].send.mock.calls.length).toBe(sends)
    expect(session.engine.started).toBe(true)
    // The project still takes commands.
    session.dispatcher.run(setParamCommand(firstParam, 0.6))
    expect(session.project.params[firstParam]).toBe(0.6)
  })

  it('leaves the MIDI input alone when the folder is refused', async () => {
    midiOn = true
    const released = vi.fn(() => Promise.resolve())
    midiReleaseAll = released
    const session = newSession()
    await session.dispatcher.start()
    const midi = (session.dispatcher as unknown as { midi: { select: ReturnType<typeof vi.fn> } }).midi
    midi.select.mockClear()
    expect(await session.dispatcher.openProject('p', [{ path: 'x.txt', bytes: new Uint8Array([1]) }])).toMatchObject({ status: 'refused' })
    expect(await session.dispatcher.openProject('p', [{ path: '../x', bytes: new Uint8Array() }])).toMatchObject({ status: 'refused' })
    expect(released).not.toHaveBeenCalled()
    expect(midi.select).not.toHaveBeenCalled()
  })

  it('shows a storage error and a too large file as unreadable, not as damaged', async () => {
    const { refusalNotice } = await import('./open')
    expect(refusalNotice('io')).toBe('project.open.unreadable')
    expect(refusalNotice('too_large')).toBe('project.open.unreadable')
    expect(refusalNotice('corrupt')).toBe('project.open.invalid')
  })

  it('plays an opened project in the page when Boost is on, since the helper cannot hold a project yet', async () => {
    const native = boostEngine()
    localStorage.setItem(TOKEN_KEY, 'x')
    const session = newSession()
    await session.dispatcher.start()
    await session.dispatcher.enterBoost()
    expect(session.engine.mode).toBe('native')
    const sent = native.send.mock.calls.length
    const pushes = webs[0].push.mock.calls.length

    expect(await session.dispatcher.openProject('song', song())).toEqual({ status: 'opened', name: 'Song "one"' })
    // The helper was never told: it would have refused the command.
    expect(native.send.mock.calls.slice(sent).some(([c]) => c.type === 'project.open')).toBe(false)
    expect(native.close).toHaveBeenCalled()
    expect(session.engine.mode).toBe('web')
    expect(session.engine.notice).toBe('project.open.webOnly')
    // The Web engine got the snapshot of the opened project.
    expect(webs[0].push.mock.calls.length).toBe(pushes + 1)
    expect(session.project.view?.project.name).toBe('Song "one"')
    expect(Object.keys(session.project.view!.slots.tracks)).toHaveLength(2)
  })

  it('stops the Web engine when the snapshot of the opened project does not encode', async () => {
    const session = newSession()
    await session.dispatcher.start()
    vi.spyOn(console, 'error').mockImplementation(() => {})
    // The core applied the opening but reports that the engine bytes did not encode.
    const open = vi.spyOn(ProjectCore.prototype, 'open').mockImplementationOnce(function (this: ProjectCore, name: string) {
      const result = this.run({ type: 'project.open', folder: name })
      if (result.report.status === 'applied') result.report.stream_error = -3
      return result
    })
    expect((await session.dispatcher.openProject('song', song())).status).toBe('opened')
    expect(session.engine.streamError).toEqual({ code: -3, at: 0 })
    expect(session.engine.started).toBe(false)
    expect(session.project.view?.project.name).toBe('Song "one"')
    open.mockRestore()
  })
})

describe('the project in a folder (SV-2)', () => {
  beforeEach(() => {
    webs = []
    nativeEngine = null
    midiOn = false
    midiReleaseAll = () => Promise.resolve()
    localStorage.clear()
  })

  it('says whether the project has changes its folder lacks, and gives the writes', async () => {
    const session = newSession()
    expect(session.dispatcher.host.dirty()).toBeNull()
    expect(session.dispatcher.host.save('changed')).toBeNull()
    await session.dispatcher.start()
    expect(session.dispatcher.host.dirty()).toBe(false)
    session.dispatcher.run(setParamCommand(firstParam, 0.3))
    expect(session.dispatcher.host.dirty()).toBe(true)
    const frame = session.dispatcher.host.save('everything')
    expect(frame?.status === 'ok' && frame.entries.some((e) => e.path === 'project.json')).toBe(true)
    session.dispatcher.host.saved(true)
    expect(session.dispatcher.host.dirty()).toBe(false)
    expect(session.dispatcher.host.projectId()).toBe(session.project.view!.project.id)
  })

  it('in Boost the storage commands stay in the page: the helper is never sent a command it does not know', async () => {
    nativeEngine = {
      kind: 'native',
      status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
      send: vi.fn(),
      close: vi.fn(() => Promise.resolve()),
    }
    localStorage.setItem(TOKEN_KEY, 'x')
    const session = newSession()
    await session.dispatcher.start()
    await session.dispatcher.enterBoost()
    vi.spyOn(session.storage, 'save').mockResolvedValue(true)
    vi.spyOn(session.storage, 'saveAs').mockResolvedValue(true)
    vi.spyOn(session.storage, 'exportPack').mockResolvedValue(true)
    vi.spyOn(session.storage, 'importPack').mockResolvedValue(true)
    const newProject = vi.spyOn(session.storage, 'newProject').mockResolvedValue(true)
    const sent = nativeEngine.send.mock.calls.length
    session.dispatcher.run({ type: 'project.save' })
    session.dispatcher.run({ type: 'project.save_as' })
    session.dispatcher.run({ type: 'project.export_pack' })
    session.dispatcher.run({ type: 'project.import_pack' })
    session.dispatcher.run({ type: 'project.new' })
    expect(nativeEngine.send.mock.calls.length).toBe(sent)
    expect(session.storage.save).toHaveBeenCalledTimes(1)
    expect(session.storage.exportPack).toHaveBeenCalledTimes(1)
    expect(session.storage.importPack).toHaveBeenCalledTimes(1)
    expect(newProject).toHaveBeenCalledTimes(1)
    // A command for the engine still reaches the helper.
    session.dispatcher.run(setParamCommand(firstParam, 0.6))
    expect(nativeEngine.send.mock.calls.at(-1)![0]).toMatchObject({ type: 'device.set_param' })
  })

  it('packs the project of the core and unpacks the files of a pack, before audio starts', async () => {
    const session = newSession()
    await session.dispatcher.openProject('song', readFolder(SONG_FOLDER))
    const frame = session.dispatcher.host.save('everything')
    if (frame?.status !== 'ok') throw new Error('no frame')
    const files = frame.entries.flatMap((e) => (e.type === 'write' ? [{ path: e.path, bytes: e.bytes }] : []))
    expect(session.dispatcher.host.projectName()).toBe(session.project.view!.project.name)
    const packed = await session.dispatcher.host.packExport(files)
    if (packed.status !== 'ok') throw new Error(packed.message)
    const unpacked = await session.dispatcher.host.packImport(packed.zip)
    expect(unpacked.status === 'ok' && unpacked.files.some((f) => f.path === 'project.json')).toBe(true)
  })

  it('a copy takes a new identity and the page sees it at once (the lock of the copy is asked for under it)', async () => {
    const session = newSession()
    await session.dispatcher.start()
    const first = session.dispatcher.host.projectId()
    session.dispatcher.host.save('everything')
    session.dispatcher.host.saved(true)
    expect(session.dispatcher.host.projectId()).toBe(first)
    session.dispatcher.host.save('copy')
    expect(session.dispatcher.host.projectId()).not.toBe(first)
    expect(session.dispatcher.host.projectId()).toBe(session.dispatcher.host.projectId())
  })

  it('tells the storage after every command, and after opening or starting a project', async () => {
    const session = newSession()
    await session.dispatcher.start()
    const changed = vi.spyOn(session.storage, 'changed')
    session.dispatcher.run(setParamCommand(firstParam, 0.3))
    expect(changed).toHaveBeenCalledTimes(1)
    await session.dispatcher.openProject('song', readFolder(SONG_FOLDER))
    expect(changed).toHaveBeenCalledTimes(2)
    await session.dispatcher.newProject()
    expect(changed).toHaveBeenCalledTimes(3)
  })

  it('runs Save and Save as even when no engine plays, and nothing else', async () => {
    const session = newSession()
    // A core without an engine: a project opened before audio starts.
    await session.dispatcher.openProject('song', readFolder(SONG_FOLDER))
    const save = vi.spyOn(session.storage, 'save').mockResolvedValue(true)
    const saveAs = vi.spyOn(session.storage, 'saveAs').mockResolvedValue(true)
    const exportPack = vi.spyOn(session.storage, 'exportPack').mockResolvedValue(true)
    const importPack = vi.spyOn(session.storage, 'importPack').mockResolvedValue(true)
    session.dispatcher.run({ type: 'project.save' })
    session.dispatcher.run({ type: 'project.save_as' })
    session.dispatcher.run({ type: 'project.export_pack' })
    session.dispatcher.run({ type: 'project.import_pack' })
    expect(save).toHaveBeenCalledTimes(1)
    expect(saveAs).toHaveBeenCalledTimes(1)
    expect(exportPack).toHaveBeenCalledTimes(1)
    expect(importPack).toHaveBeenCalledTimes(1)
    const name = session.project.view!.project.name
    session.dispatcher.run({ type: 'project.rename', name: 'Not applied' })
    expect(session.project.view!.project.name).toBe(name)
  })

  it('the export is a command of the page: it opens the dialog with its settings, even without an engine, and never reaches the helper', async () => {
    nativeEngine = {
      kind: 'native',
      status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
      send: vi.fn(),
      close: vi.fn(() => Promise.resolve()),
    }
    localStorage.setItem(TOKEN_KEY, 'x')
    const session = newSession()
    // A core without an engine: a project opened before audio starts.
    await session.dispatcher.openProject('song', readFolder(SONG_FOLDER))
    session.dispatcher.run({ type: 'project.export_audio', bit_depth: 'pcm16', sample_rate: 44100 })
    expect(session.audioExport.open).toBe(true)
    expect(session.audioExport.choices).toMatchObject({ bitDepth: 'pcm16', sampleRate: 44_100, region: 'song' })
    session.audioExport.close()
    // In Boost the helper is not sent it.
    await session.dispatcher.start()
    await session.dispatcher.enterBoost()
    const sent = nativeEngine.send.mock.calls.length
    session.dispatcher.run({ type: 'project.export_audio' })
    expect(nativeEngine.send.mock.calls.length).toBe(sent)
    expect(session.audioExport.open).toBe(true)
  })

  it('tells the project store about every stop, seek and pause, and about no other transport command (#394)', async () => {
    const session = newSession()
    await session.dispatcher.start()
    const cued = vi.fn()
    session.project.cued = cued
    session.dispatcher.run({ type: 'transport.stop' })
    session.dispatcher.run({ type: 'transport.seek', tick: 0 })
    session.dispatcher.run({ type: 'transport.pause', tick: 0 })
    expect(cued).toHaveBeenCalledTimes(3)
    session.dispatcher.run({ type: 'transport.set_loop', loop_region: { start: 0, end: 3840 } })
    expect(cued).toHaveBeenCalledTimes(3)
  })

  it('knows what an export can render: a clip in the arrangement, a loop region', async () => {
    const session = newSession()
    await session.dispatcher.start()
    expect(session.audioExport.hasContent).toBe(false)
    expect(session.audioExport.hasLoop).toBe(false)
    session.dispatcher.run({ type: 'clip.create', track: SINGLE_SYNTH.track, at: { at: 'arrangement', start: 0 } })
    session.dispatcher.run({ type: 'transport.set_loop', loop_region: { start: 0, end: 3840 } })
    expect(session.audioExport.hasContent).toBe(true)
    expect(session.audioExport.hasLoop).toBe(true)
    // The core's job for the dialog's settings: ready for the song and for the loop.
    expect(session.dispatcher.exportHost.job({ region: 'song' })?.status).toBe('ready')
    expect(session.dispatcher.exportHost.job({ region: 'loop' })?.status).toBe('ready')
  })

  it('starts a new project: the default one, its own identity, the history empty, the engine started over', async () => {
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run(setParamCommand(firstParam, 0.3))
    const before = session.project.view!
    const pushes = webs[0].push.mock.calls.length
    expect(await session.dispatcher.newProject()).toBe(true)
    const after = session.project.view!
    expect(after.project.id).not.toBe(before.project.id)
    expect(session.project.canUndo).toBe(false)
    expect(session.project.params[firstParam]).toBe(DEFAULT_PARAMS[firstParam])
    expect(webs[0].push.mock.calls.length).toBe(pushes + 1)
    expect(after.slots.epoch).toBe(before.slots.epoch + 1)
    expect(session.dispatcher.host.dirty()).toBe(false)
  })

  it('starts a new project before audio starts, and the first engine then plays it', async () => {
    const session = newSession()
    expect(await session.dispatcher.newProject()).toBe(true)
    // Named after the moment it started (#473), never "Untitled".
    expect(session.project.view?.project.name).toMatch(/^Sketch /)
    await session.dispatcher.start()
    expect(webs[0].push).toHaveBeenCalledTimes(1)
  })

  it('a project.new run as a command goes through the storage, which asks before the core replaces anything (#346)', async () => {
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run(setParamCommand(firstParam, 0.6))
    expect(session.dispatcher.host.projectDirty()).toBe(true)
    const pushes = webs[0].push.mock.calls.length
    session.dispatcher.run({ type: 'project.new' })
    await vi.waitFor(() => expect(session.storage.confirm).toEqual({ action: 'new', name: null }))
    // Asked, and nothing replaced yet: the changes are still in the project.
    expect(session.project.params[firstParam]).toBe(0.6)
    expect(webs[0].push.mock.calls.length).toBe(pushes)
    // Cancel changes nothing, and the question is gone.
    session.storage.respond(false)
    await vi.waitFor(() => expect(session.storage.confirm).toBeNull())
    expect(session.project.params[firstParam]).toBe(0.6)
    expect(session.dispatcher.host.projectDirty()).toBe(true)
    expect(webs[0].push.mock.calls.length).toBe(pushes)
    // Confirm replaces the project.
    session.dispatcher.run({ type: 'project.new' })
    await vi.waitFor(() => expect(session.storage.confirm).not.toBeNull())
    session.storage.respond(true)
    await vi.waitFor(() => expect(session.project.params[firstParam]).toBe(DEFAULT_PARAMS[firstParam]))
    expect(session.dispatcher.host.dirty()).toBe(false)
    expect(webs[0].push.mock.calls.length).toBe(pushes + 1)
    expect(session.storage.confirm).toBeNull()
  })

  it('a project.new takes the name an agent gives it', async () => {
    const session = newSession()
    session.dispatcher.run({ type: 'project.new', name: 'Night drive' })
    await vi.waitFor(() => expect(session.project.view?.project.name).toBe('Night drive'))
  })

  it('a project.new on a project with nothing to lose asks nothing, and works before audio starts', async () => {
    const session = newSession()
    session.dispatcher.run({ type: 'project.new' })
    await vi.waitFor(() => expect(session.project.view?.project.name).toMatch(/^Sketch /))
    expect(session.storage.confirm).toBeNull()
  })

  it('two project.new in a row ask one question after the other, and the second sees the first one done', async () => {
    const session = newSession()
    await session.dispatcher.start()
    session.dispatcher.run(setParamCommand(firstParam, 0.6))
    session.dispatcher.run({ type: 'project.new' })
    session.dispatcher.run({ type: 'project.new' })
    await vi.waitFor(() => expect(session.storage.confirm).not.toBeNull())
    session.storage.respond(true)
    // The first replaced the project: nothing is left to lose, so the second asks nothing.
    await vi.waitFor(() => expect(session.project.params[firstParam]).toBe(DEFAULT_PARAMS[firstParam]))
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(session.storage.confirm).toBeNull()
  })
})

describe('the commands for the projects the page keeps (#323)', () => {
  beforeEach(() => {
    webs = []
    nativeEngine = null
    midiOn = false
    midiReleaseAll = () => Promise.resolve()
    localStorage.clear()
  })

  const ids = (name: string) => `${name}-id`

  it('each command reaches the storage action it names, with its arguments, even when no engine plays', async () => {
    const session = newSession()
    await session.dispatcher.openProject('song', readFolder(SONG_FOLDER))
    const spies = {
      reopenLast: vi.spyOn(session.storage, 'reopenLast').mockResolvedValue(true),
      openFolder: vi.spyOn(session.storage, 'openFolder').mockResolvedValue(true),
      openRecent: vi.spyOn(session.storage, 'openRecent').mockResolvedValue(true),
      openBrowserProject: vi.spyOn(session.storage, 'openBrowserProject').mockResolvedValue(true),
      deleteBrowserProject: vi.spyOn(session.storage, 'deleteBrowserProject').mockResolvedValue(true),
      keepMine: vi.spyOn(session.storage, 'keepMine').mockResolvedValue(true),
      loadDisk: vi.spyOn(session.storage, 'loadDisk').mockResolvedValue(true),
      resume: vi.spyOn(session.storage, 'resume').mockResolvedValue(true),
    }
    session.dispatcher.run({ type: 'project.reopen_last' })
    session.dispatcher.run({ type: 'project.open_folder' })
    session.dispatcher.run({ type: 'project.open_recent', id: ids('r') })
    session.dispatcher.run({ type: 'project.open_browser_project', dir: ids('d') })
    session.dispatcher.run({ type: 'project.delete_browser_project', dir: ids('x') })
    session.dispatcher.run({ type: 'project.keep_mine' })
    session.dispatcher.run({ type: 'project.load_folder_version' })
    session.dispatcher.run({ type: 'project.allow_access' })
    expect(spies.reopenLast).toHaveBeenCalledTimes(1)
    expect(spies.openFolder).toHaveBeenCalledTimes(1)
    expect(spies.openRecent).toHaveBeenCalledWith('r-id')
    expect(spies.openBrowserProject).toHaveBeenCalledWith('d-id')
    expect(spies.deleteBrowserProject).toHaveBeenCalledWith('x-id')
    expect(spies.keepMine).toHaveBeenCalledTimes(1)
    expect(spies.loadDisk).toHaveBeenCalledTimes(1)
    expect(spies.resume).toHaveBeenCalledTimes(1)
  })

  it('a command the project refuses (a name that names nothing) never reaches the storage', async () => {
    const session = newSession()
    await session.dispatcher.openProject('song', readFolder(SONG_FOLDER))
    const open = vi.spyOn(session.storage, 'openBrowserProject').mockResolvedValue(true)
    const remove = vi.spyOn(session.storage, 'deleteBrowserProject').mockResolvedValue(true)
    const recent = vi.spyOn(session.storage, 'openRecent').mockResolvedValue(true)
    session.dispatcher.run({ type: 'project.open_browser_project', dir: '' }, { quiet: true })
    session.dispatcher.run({ type: 'project.delete_browser_project', dir: 'x'.repeat(129) }, { quiet: true })
    session.dispatcher.run({ type: 'project.open_recent', id: '' }, { quiet: true })
    expect(open).not.toHaveBeenCalled()
    expect(remove).not.toHaveBeenCalled()
    expect(recent).not.toHaveBeenCalled()
  })

  it('on a fresh page, before any project core exists, the commands that open a project make it and still run', async () => {
    const session = newSession()
    const reopen = vi.spyOn(session.storage, 'reopenLast').mockResolvedValue(true)
    const remove = vi.spyOn(session.storage, 'deleteBrowserProject').mockResolvedValue(true)
    expect(session.dispatcher.host.dirty()).toBeNull()
    session.dispatcher.run({ type: 'project.reopen_last' })
    session.dispatcher.run({ type: 'project.delete_browser_project', dir: 'draft' })
    await vi.waitFor(() => expect(reopen).toHaveBeenCalledTimes(1))
    await vi.waitFor(() => expect(remove).toHaveBeenCalledWith('draft'))
    expect(session.dispatcher.host.dirty()).toBe(false)
    // A command that needs a project is still dropped by a page that has none.
    const fresh = newSession()
    const keep = vi.spyOn(fresh.storage, 'keepMine').mockResolvedValue(true)
    fresh.dispatcher.run({ type: 'project.keep_mine' })
    await Promise.resolve()
    expect(keep).not.toHaveBeenCalled()
  })

  it('in Boost these commands stay in the page too', async () => {
    nativeEngine = {
      kind: 'native',
      status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 },
      send: vi.fn(),
      close: vi.fn(() => Promise.resolve()),
    }
    localStorage.setItem(TOKEN_KEY, 'x')
    const session = newSession()
    await session.dispatcher.start()
    await session.dispatcher.enterBoost()
    vi.spyOn(session.storage, 'reopenLast').mockResolvedValue(true)
    vi.spyOn(session.storage, 'keepMine').mockResolvedValue(true)
    vi.spyOn(session.storage, 'resume').mockResolvedValue(true)
    const sent = nativeEngine.send.mock.calls.length
    session.dispatcher.run({ type: 'project.reopen_last' })
    session.dispatcher.run({ type: 'project.keep_mine' })
    session.dispatcher.run({ type: 'project.allow_access' })
    expect(nativeEngine.send.mock.calls.length).toBe(sent)
    expect(session.storage.reopenLast).toHaveBeenCalledTimes(1)
  })

  it('project.list_stored answers with what the storage read, and an empty list is not an old one', async () => {
    const session = newSession()
    const handle = { name: 'last' } as unknown as FileSystemDirectoryHandle
    vi.spyOn(session.storage, 'refreshStored').mockImplementation(async () => {
      session.storage.recent = [{ id: 'r1', name: 'last', handle, at: 1 }]
      session.storage.browserProjects = [{ dir: 'd1', id: 'p1', name: 'Draft', modified: 2 }]
    })
    expect(await session.dispatcher.listStored()).toEqual({
      recent: [{ id: 'r1', name: 'last' }],
      browser: [{ dir: 'd1', name: 'Draft' }],
    })
    vi.spyOn(session.storage, 'refreshStored').mockImplementation(async () => {
      session.storage.recent = []
      session.storage.browserProjects = []
    })
    expect(await session.dispatcher.listStored()).toEqual({ recent: [], browser: [] })
    // Through `run`, the command asks the storage to read again.
    const run = vi.spyOn(session.storage, 'refreshStored')
    session.dispatcher.run({ type: 'project.list_stored' })
    await vi.waitFor(() => expect(run).toHaveBeenCalled())
  })
})
