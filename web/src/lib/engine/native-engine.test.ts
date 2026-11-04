import { afterEach, describe, expect, it, vi } from 'vitest'
import { MAX_STREAM_BYTES, PROTOCOL_VERSION } from '../generated/protocol'
import type { EngineEvents } from './engine'
import { FEEDBACK_HEADER_BYTES, FEEDBACK_VERSION, TRACK_METER_BYTES } from './feedback'
import { helperDeviceMessage, NativeEngine, NoOutputError, STREAM_TOO_LARGE } from './native-engine'

class FakeSocket {
  static last: FakeSocket
  static OPEN = 1
  readyState = 1
  binaryType: BinaryType = 'blob'
  onmessage: ((event: { data: unknown }) => void) | null = null
  onclose: (() => void) | null = null
  sent: (string | Uint8Array)[] = []
  constructor(readonly url: string) {
    FakeSocket.last = this
  }
  send(data: string | Uint8Array) {
    this.sent.push(data)
  }
  close() {}
}

function frame(): ArrayBuffer {
  const buffer = new ArrayBuffer(FEEDBACK_HEADER_BYTES + 2 * TRACK_METER_BYTES)
  const view = new DataView(buffer)
  view.setUint8(0, FEEDBACK_VERSION)
  view.setUint32(4, 2, true)
  view.setUint32(40, 1, true)
  view.setFloat32(FEEDBACK_HEADER_BYTES, 0.5, true)
  view.setFloat32(FEEDBACK_HEADER_BYTES + TRACK_METER_BYTES, 0.75, true)
  return buffer
}

afterEach(() => vi.unstubAllGlobals())

describe('the Boost engine reads the helper', () => {
  it('asks for ArrayBuffer frames and hands a binary report to onFeedback', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn() } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    const socket = FakeSocket.last
    expect(socket.binaryType).toBe('arraybuffer')
    socket.onmessage!({ data: JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000 }) })
    await connected
    socket.onmessage!({ data: frame() })
    expect(events.onFeedback).toHaveBeenCalledTimes(1)
    const report = events.onFeedback.mock.calls[0][0]
    expect(report.tracks[0].peakL).toBe(0.5)
    expect(report.master.peakL).toBe(0.75)
    // A frame of another version, a truncated one and a text that is not a message: ignored.
    const other = frame()
    new DataView(other).setUint8(0, 1)
    socket.onmessage!({ data: other })
    socket.onmessage!({ data: frame().slice(0, 40) })
    socket.onmessage!({ data: 'not json' })
    expect(events.onFeedback).toHaveBeenCalledTimes(1)
    expect(events.onError).not.toHaveBeenCalled()
  })

  it('keeps the helper update state and hands each change to onHelperUpdate (#515)', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn(), onHelperUpdate: vi.fn() } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    const socket = FakeSocket.last
    socket.onmessage!({ data: JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000 }) })
    // The first state arrives right after the hello, before the page made this engine current.
    socket.onmessage!({
      data: JSON.stringify({ type: 'update.status', current: '0.1.11', checked_at: null, status: { state: 'downloading', version: '0.1.12', percent: 40 } }),
    })
    const engine = await connected
    expect(engine.update).toEqual({ current: '0.1.11', checkedAt: null, status: { state: 'downloading', version: '0.1.12', percent: 40 } })
    socket.onmessage!({ data: JSON.stringify({ type: 'update.status', checked_at: 1793665000, status: { state: 'ready', version: '0.1.12' } }) })
    expect(engine.update).toEqual({ current: null, checkedAt: 1793665000, status: { state: 'ready', version: '0.1.12' } })
    expect(events.onHelperUpdate).toHaveBeenCalledTimes(2)
    expect(events.onHelperUpdate.mock.calls[1][0]).toEqual(engine.update)
    expect(events.onError).not.toHaveBeenCalled()
  })

  it('hands an agent call to onAgentCall and answers it once, with its id (AG-1)', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const onAgentCall = vi.fn()
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn(), onAgentCall } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    const socket = FakeSocket.last
    socket.onmessage!({ data: JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000 }) })
    await connected
    socket.onmessage!({ data: JSON.stringify({ type: 'agent.call', id: 7, agent: 'claude-code', command: { type: 'history.undo' } }) })
    expect(onAgentCall).toHaveBeenCalledWith('claude-code', { type: 'history.undo' }, expect.any(Function), expect.any(AbortSignal))
    const answer = onAgentCall.mock.calls[0][2]
    answer({ status: 'applied', output: { type: 'history.undo' } })
    answer({ status: 'invalid', message: 'twice' })
    const results = socket.sent.filter((m): m is string => typeof m === 'string').map((m) => JSON.parse(m))
    expect(results).toEqual([{ type: 'agent.result', id: 7, result: { status: 'applied', output: { type: 'history.undo' } } }])
  })

  it("aborts its agents' calls when the session ends: their questions are withdrawn (AG-4)", async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const onAgentCall = vi.fn()
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn(), onAgentCall } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    const socket = FakeSocket.last
    socket.onmessage!({ data: JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000 }) })
    await connected
    socket.onmessage!({ data: JSON.stringify({ type: 'agent.call', id: 7, agent: 'a', command: { type: 'project.new' } }) })
    const signal = onAgentCall.mock.calls[0][3] as AbortSignal
    expect(signal.aborted).toBe(false)
    socket.onclose!()
    expect(signal.aborted).toBe(true)
  })

  it('answers an agent call unavailable when the page takes none, never leaving the agent waiting', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn() } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    const socket = FakeSocket.last
    socket.onmessage!({ data: JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000 }) })
    await connected
    socket.onmessage!({ data: JSON.stringify({ type: 'agent.call', id: 3, agent: 'a', command: { type: 'history.undo' } }) })
    const sent = JSON.parse(socket.sent.at(-1) as string)
    expect(sent).toMatchObject({ type: 'agent.result', id: 3, result: { status: 'unavailable' } })
  })

  it('takes a later hello as the status of the output the helper reopened (#577)', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn() } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    const socket = FakeSocket.last
    socket.onmessage!({ data: JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000, estimated_latency_ms: 12 }) })
    const engine = await connected
    // The output was replaced and the session plays on: a new hello at the new output's rate.
    socket.onmessage!({ data: JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 44100, buffer_frames: 441, estimated_latency_ms: 20 }) })
    expect(engine.status).toEqual({ kind: 'native', sampleRate: 44100, estimatedLatencyMs: 20 })
    expect(events.onStatus).toHaveBeenCalledTimes(2)
    expect(events.onStatus.mock.calls[1][0]).toEqual(engine.status)
    expect(events.onError).not.toHaveBeenCalled()
    expect(events.onClosed).not.toHaveBeenCalled()
  })

  it('rejects with NoOutputError when the helper has no audio output (#525)', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn() } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    const socket = FakeSocket.last
    socket.onmessage!({ data: JSON.stringify({ type: 'error', code: 'audio.no_output', message: 'the helper has no audio output' }) })
    socket.onclose!()
    await expect(connected).rejects.toBeInstanceOf(NoOutputError)
    expect(events.onError).not.toHaveBeenCalled()
    expect(events.onClosed).not.toHaveBeenCalled()
  })

  it('takes an output lost before the hello for no output (#525)', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn() } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    FakeSocket.last.onmessage!({ data: JSON.stringify({ type: 'error', code: 'audio.stream', message: 'the audio output stream reported an error' }) })
    await expect(connected).rejects.toBeInstanceOf(NoOutputError)
  })

  /** A session whose hello announced `epoch`, and its events. */
  async function session(epoch = 0) {
    vi.stubGlobal('WebSocket', FakeSocket)
    const events = {
      onStatus: vi.fn(),
      onFeedback: vi.fn(),
      onError: vi.fn(),
      onClosed: vi.fn(),
      onStreamRefused: vi.fn(),
      onMidiMessage: vi.fn(),
    } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    const socket = FakeSocket.last
    socket.onmessage!({
      data: JSON.stringify({ type: 'hello', protocol: PROTOCOL_VERSION, sample_rate: 48000, epoch, capabilities: { midi: true, disk: false } }),
    })
    return { engine: await connected, socket, events }
  }

  it('keeps the epoch and the capabilities of the hello (BO-1)', async () => {
    const { engine } = await session(41)
    expect(engine.epoch).toBe(41)
    expect(engine.capabilities).toEqual({ midi: true, disk: false })
  })

  it('sends the engine bytes of a command in a binary frame, and only the MIDI input commands as messages (BO-1)', async () => {
    const { engine, socket } = await session()
    const bytes = new Uint8Array([1, 0, 90, 0])
    engine.send({ type: 'live.note_on', track: 't', note: 60, velocity: 100 }, bytes)
    expect(socket.sent).toEqual([bytes])
    // A command with no engine bytes sends nothing.
    engine.send({ type: 'project.save' }, new Uint8Array())
    expect(socket.sent).toHaveLength(1)
    // The helper's MIDI inputs: their message, beside the bytes (none here).
    engine.send({ type: 'midi.select_input', id: 'a' }, new Uint8Array())
    engine.send({ type: 'midi.list_inputs' }, new Uint8Array())
    expect(socket.sent.slice(1)).toEqual([JSON.stringify({ type: 'midi.select_input', id: 'a' }), JSON.stringify({ type: 'midi.list_inputs' })])
    expect(helperDeviceMessage({ type: 'midi.select_input' })).toEqual({ type: 'midi.select_input', id: null })
    expect(helperDeviceMessage({ type: 'live.all_notes_off' })).toBeNull()
    // Closing frees the MIDI input and sends no command.
    await engine.close()
    expect(socket.sent.at(-1)).toBe(JSON.stringify({ type: 'midi.select_input', id: null }))
  })

  it('never sends a stream longer than the helper reads: it is reported refused', async () => {
    const { engine, socket, events } = await session(7)
    const huge = { length: MAX_STREAM_BYTES + 1 } as Uint8Array
    engine.push(huge)
    expect(socket.sent).toEqual([])
    expect(events.onStreamRefused).toHaveBeenCalledWith(STREAM_TOO_LARGE, 0, 7)
  })

  it('hands a refused stream and a MIDI message of the helper to the page', async () => {
    const { socket, events } = await session()
    socket.onmessage!({ data: JSON.stringify({ type: 'stream_refused', code: -6, at: 0, epoch: 12 }) })
    expect(events.onStreamRefused).toHaveBeenCalledWith(-6, 0, 12)
    socket.onmessage!({ data: JSON.stringify({ type: 'midi.message', data: [0x90, 60, 100], played: true }) })
    expect(events.onMidiMessage).toHaveBeenCalledWith([0x90, 60, 100], true)
    socket.onmessage!({ data: JSON.stringify({ type: 'midi.message', data: [0xb0, 1, 64], played: false }) })
    expect(events.onMidiMessage).toHaveBeenLastCalledWith([0xb0, 1, 64], false)
    expect(events.onError).not.toHaveBeenCalled()
  })

  it('sends the table of the helper MIDI input only when it changed (#598)', async () => {
    const { engine, socket } = await session(3)
    engine.route({ epoch: 4, tracks: [0, 2] })
    expect(socket.sent).toEqual([JSON.stringify({ type: 'live.routing', epoch: 4, tracks: [0, 2] })])
    engine.route({ epoch: 4, tracks: [0, 2] })
    expect(socket.sent).toHaveLength(1)
    engine.route({ epoch: 4, tracks: [0] })
    engine.route({ epoch: 5, tracks: [0] })
    engine.route({ epoch: 5, tracks: [] })
    expect(socket.sent.slice(1)).toEqual([
      JSON.stringify({ type: 'live.routing', epoch: 4, tracks: [0] }),
      JSON.stringify({ type: 'live.routing', epoch: 5, tracks: [0] }),
      JSON.stringify({ type: 'live.routing', epoch: 5, tracks: [] }),
    ])
    // A new session sends its first table even when it is the same.
    const next = await session(3)
    next.engine.route({ epoch: 5, tracks: [] })
    expect(next.socket.sent).toHaveLength(1)
  })

  it('takes a session closed without a word for a refused one', async () => {
    vi.stubGlobal('WebSocket', FakeSocket)
    const events = { onStatus: vi.fn(), onFeedback: vi.fn(), onError: vi.fn(), onClosed: vi.fn() } satisfies EngineEvents
    const connected = NativeEngine.connect('t', events)
    FakeSocket.last.onclose!()
    const error = await connected.catch((e: unknown) => e)
    expect(error).toBeInstanceOf(Error)
    expect(error).not.toBeInstanceOf(NoOutputError)
  })
})
