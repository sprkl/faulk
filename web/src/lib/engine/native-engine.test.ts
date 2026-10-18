import { afterEach, describe, expect, it, vi } from 'vitest'
import { PROTOCOL_VERSION } from '../generated/protocol'
import type { EngineEvents } from './engine'
import { FEEDBACK_HEADER_BYTES, FEEDBACK_VERSION, TRACK_METER_BYTES } from './feedback'
import { NativeEngine } from './native-engine'

class FakeSocket {
  static last: FakeSocket
  static OPEN = 1
  readyState = 1
  binaryType: BinaryType = 'blob'
  onmessage: ((event: { data: unknown }) => void) | null = null
  onclose: (() => void) | null = null
  sent: string[] = []
  constructor(readonly url: string) {
    FakeSocket.last = this
  }
  send(data: string) {
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
})
