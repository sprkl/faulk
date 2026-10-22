// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NativeEngine } from '../engine/native-engine'
import type { EngineEvents } from '../engine/engine'
import type { Command } from '../generated/protocol'
import { DEFAULT_PARAMS } from '../params'
import { CoreError, ProjectCore } from '../project/core'
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
  suspend: ReturnType<typeof vi.fn>
  resume: ReturnType<typeof vi.fn>
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
  coreModule: () =>
    import('node:fs').then((fs) =>
      WebAssembly.compile(fs.readFileSync(`${process.cwd()}/src/lib/project/wasm/project.wasm`)),
    ),
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

const newSession = () => createStores('play', 'devices')

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
    session.dispatcher.onLocalAccessGranted()
    await vi.waitFor(() => expect(session.engine.helper).toBe('available'))
    expect(probeHelper).toHaveBeenCalledTimes(2)
  })

  it('lets the newer probe decide when the probe behind the prompt gives up later', async () => {
    const session = newSession()
    let giveUp: (p: Probe) => void = () => {}
    probeHelper.mockReturnValueOnce(new Promise<Probe>((r) => (giveUp = r)))
    const first = session.dispatcher.checkHelper()
    expect(session.engine.helper).toBe('checking')
    session.dispatcher.onLocalAccessGranted()
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
    session.dispatcher.onLocalAccessGranted()
    answer({ status: 'available' })
    expect(await older).toBe('available')
    expect(session.engine.helper).toBe('checking')
  })

  it('does not probe when the helper is already found', async () => {
    const session = newSession()
    await session.dispatcher.checkHelper()
    expect(session.engine.helper).toBe('available')
    session.dispatcher.onLocalAccessGranted()
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
const setParamCommand = (param: string, value: number) =>
  ({ type: 'device.set_param', device: SYNTH_DEVICE, param, value }) as Command
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

  it('keeps the helper in step in Boost mode and catches the Web engine up by snapshot on return', async () => {
    nativeEngine = { kind: 'native', status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 }, send: vi.fn(), close: vi.fn(() => Promise.resolve()) }
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
    nativeEngine = { kind: 'native', status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 }, send: vi.fn(), close: vi.fn(() => Promise.resolve()) }
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
    nativeEngine = { kind: 'native', status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 }, send: vi.fn(), close: vi.fn(() => Promise.resolve()) }
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
      const connect = vi
        .spyOn(NativeEngine, 'connect')
        .mockImplementationOnce(() => new Promise((_, reject) => (refuse = reject)))
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
    nativeEngine = { kind: 'native', status: { kind: 'native', sampleRate: 48000, estimatedLatencyMs: 5 }, send: vi.fn(), close: vi.fn(() => Promise.resolve()) }
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
    expect(source).not.toMatch(/command\.type|type: '|'(project|track|device|clip|note|scene|launch|arrangement|automation|transport|mixer|record|midi(?!.connect)|live|history|view)\.[a-z_.]+'/)
    expect(source).not.toMatch(/from '\.\.\/synth-commands'/)
  })
})
