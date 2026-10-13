// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const pairHelper = vi.fn()
vi.mock('./engine/helper-client', () => ({
  pairHelper: (code: string) => pairHelper(code),
  probeHelper: () => Promise.resolve({ status: 'available' }),
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
  const location = { hash, pathname: '/', search: '' } as Location
  const history = { state: null, replaceState: vi.fn() } as unknown as History
  return { location, history }
}

describe('pairing link in the session', () => {
  beforeEach(() => {
    pairHelper.mockReset()
    pairHelper.mockResolvedValue({ status: 'refused', attemptsLeft: 4 })
  })

  it('keeps the code until audio starts, then pairs once', async () => {
    const session = new Session()
    const { location, history } = address('#pair=12345678')
    session.takePairLink(location, history)
    expect(history.replaceState).toHaveBeenCalledWith(null, '', '/')
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
    const { location, history } = address('#pair=87654321')
    session.takePairLink(location, history)
    await vi.waitFor(() => expect(pairHelper).toHaveBeenCalledWith('87654321'))
    expect(session.pendingPairCode).toBeNull()
  })

  it('ignores an invalid or missing link', async () => {
    const session = new Session()
    session.takePairLink(...(Object.values(address('#pair=12')) as [Location, History]))
    session.takePairLink(...(Object.values(address('')) as [Location, History]))
    expect(session.pendingPairCode).toBeNull()
    await session.start()
    expect(pairHelper).not.toHaveBeenCalled()
  })
})
