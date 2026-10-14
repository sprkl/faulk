import { describe, expect, it, vi } from 'vitest'
import { isPairHash, pairCodeFromHash, takePairLink } from './pair-link'

describe('pairing links', () => {
  it('reads an 8-digit code from #pair=', () => {
    expect(pairCodeFromHash('#pair=12345678')).toBe('12345678')
    expect(pairCodeFromHash('#pair=04718263')).toBe('04718263')
  })

  it('refuses anything else', () => {
    for (const hash of ['', '#', '#pair=', '#pair=1234567', '#pair=123456789', '#pair=1234 5678', '#pair=abcdefgh', '#pair=12345678&x', '#other=12345678', 'pair=12345678']) {
      expect(pairCodeFromHash(hash), hash).toBeNull()
    }
    expect(isPairHash('#pair=x')).toBe(true)
    expect(isPairHash('#other')).toBe(false)
  })

  it('removes the fragment from the address and keeps path and query', () => {
    const history = { state: { a: 1 }, replaceState: vi.fn() }
    const code = takePairLink({ hash: '#pair=12345678', pathname: '/app', search: '?x=1' }, history)
    expect(code).toBe('12345678')
    expect(history.replaceState).toHaveBeenCalledWith({ a: 1 }, '', '/app?x=1')
  })

  it('also removes a malformed pairing fragment, but leaves other fragments alone', () => {
    const history = { state: null, replaceState: vi.fn() }
    expect(takePairLink({ hash: '#pair=nope', pathname: '/', search: '' }, history)).toBeNull()
    expect(history.replaceState).toHaveBeenCalledWith(null, '', '/')
    history.replaceState.mockClear()
    expect(takePairLink({ hash: '#section', pathname: '/', search: '' }, history)).toBeNull()
    expect(history.replaceState).not.toHaveBeenCalled()
  })
})
