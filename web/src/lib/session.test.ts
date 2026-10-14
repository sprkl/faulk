// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const pairHelper = vi.fn()
type Probe = { status: 'absent' } | { status: 'available' }
const probeHelper = vi.fn<() => Promise<Probe>>(() => Promise.resolve({ status: 'available' }))
vi.mock('./engine/helper-client', () => ({
  pairHelper: (code: string) => pairHelper(code),
  probeHelper: () => probeHelper(),
}))
vi.mock('./engine/web-engine', () => ({
  WebEngine: {
    create: () =>
      Promise.resolve({
        kind: 'web',
        run: vi.fn(),
        status: () => ({ sampleRate: 48000, bufferFrames: 128, estimatedLatencyMs: null }),
        suspend: vi.fn(() => Promise.resolve()),
        resume: vi.fn(() => Promise.resolve()),
      }),
  },
}))
vi.mock('./engine/native-engine', () => ({
  NativeEngine: { connect: () => Promise.reject(new Error('no helper in tests')) },
}))
vi.mock('./midi/web-midi', () => ({
  WebMidiHost: { supported: () => false },
}))

const { Session } = await import('./session.svelte')

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
    const session = new Session()
    const { location, clear } = address('#pair=12345678')
    session.takePairLink(location, clear)
    expect(clear).toHaveBeenCalledWith('/app')
    expect(session.pendingPairCode).toBe('12345678')
    expect(pairHelper).not.toHaveBeenCalled()

    await session.start()
    expect(pairHelper).toHaveBeenCalledTimes(1)
    expect(pairHelper).toHaveBeenCalledWith('12345678')
    expect(session.pendingPairCode).toBeNull()
  })

  it('pairs at once when audio already runs', async () => {
    const session = new Session()
    await session.start()
    const { location, clear } = address('#pair=87654321')
    session.takePairLink(location, clear)
    await vi.waitFor(() => expect(pairHelper).toHaveBeenCalledWith('87654321'))
    expect(session.pendingPairCode).toBeNull()
  })

  it('reports a pairing failure as such, not as an audio start failure', async () => {
    pairHelper.mockResolvedValue({ status: 'failed' })
    const session = new Session()
    const { location, clear } = address('#pair=12345678')
    session.takePairLink(location, clear)
    await session.start()
    expect(session.started).toBe(true)
    expect(session.notice).toBeNull()
    expect(session.pairingMessage).toEqual({ key: 'helper.failed' })
  })

  it('keeps the audio started when pairing throws unexpectedly', async () => {
    pairHelper.mockRejectedValue(new Error('boom'))
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const session = new Session()
    const { location, clear } = address('#pair=12345678')
    session.takePairLink(location, clear)
    await session.start()
    expect(session.started).toBe(true)
    expect(session.notice).toBeNull()
    expect(session.pairingMessage).toEqual({ key: 'helper.failed' })
  })

  it('reports a pairing failure on a link that arrives after the start', async () => {
    pairHelper.mockRejectedValue(new Error('boom'))
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const session = new Session()
    await session.start()
    const { location, clear } = address('#pair=87654321')
    session.takePairLink(location, clear)
    await vi.waitFor(() => expect(session.pairingMessage).toEqual({ key: 'helper.failed' }))
    expect(session.started).toBe(true)
  })

  it('ignores an invalid or missing link', async () => {
    const session = new Session()
    session.takePairLink(...(Object.values(address('#pair=12')) as [Location, (url: string) => void]))
    session.takePairLink(...(Object.values(address('')) as [Location, (url: string) => void]))
    expect(session.pendingPairCode).toBeNull()
    await session.start()
    expect(pairHelper).not.toHaveBeenCalled()
  })
})

describe('probing again once the browser allows local access', () => {
  beforeEach(() => {
    probeHelper.mockReset()
    probeHelper.mockResolvedValue({ status: 'available' })
  })

  it('probes again when the last probe found nothing', async () => {
    const session = new Session()
    probeHelper.mockResolvedValueOnce({ status: 'absent' })
    await session.checkHelper()
    expect(session.helper).toBe('absent')
    session.onLocalAccessGranted()
    await vi.waitFor(() => expect(session.helper).toBe('available'))
    expect(probeHelper).toHaveBeenCalledTimes(2)
  })

  it('lets the newer probe decide when the probe behind the prompt gives up later', async () => {
    const session = new Session()
    let giveUp: (p: Probe) => void = () => {}
    probeHelper.mockReturnValueOnce(new Promise<Probe>((r) => (giveUp = r)))
    const first = session.checkHelper()
    expect(session.helper).toBe('checking')
    session.onLocalAccessGranted()
    await vi.waitFor(() => expect(session.helper).toBe('available'))
    giveUp({ status: 'absent' })
    await first
    expect(session.helper).toBe('available')
  })

  it('does not probe when the helper is already found', async () => {
    const session = new Session()
    await session.checkHelper()
    expect(session.helper).toBe('available')
    session.onLocalAccessGranted()
    expect(probeHelper).toHaveBeenCalledTimes(1)
  })
})
