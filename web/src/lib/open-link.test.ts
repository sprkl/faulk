// `/app?open=<link>` (#497): the link is taken once and leaves the address at once.
import { describe, expect, it, vi } from 'vitest'
import { ENTRIES, packUrl } from './journey/entries'
import { currentLink, openLinkHref, takeOpenLink } from './open-link'

const at = (search: string, hash = '') => ({ pathname: '/app', search, hash })

describe('takeOpenLink', () => {
  it('takes the link and clears it from the address, keeping the other parameters and the fragment', () => {
    const clear = vi.fn()
    const link = 'https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/a/project.faulkpack'
    expect(takeOpenLink(at(`?lang=fr&open=${encodeURIComponent(link)}&x=1`, '#top'), clear)).toBe(link)
    expect(clear).toHaveBeenCalledWith('/app?lang=fr&x=1#top')
    const alone = vi.fn()
    expect(takeOpenLink(at(`?open=${encodeURIComponent(link)}`), alone)).toBe(link)
    expect(alone).toHaveBeenCalledWith('/app')
  })

  it('leaves an address without the parameter alone', () => {
    const clear = vi.fn()
    expect(takeOpenLink(at('?lang=fr'), clear)).toBeNull()
    expect(takeOpenLink(at(''), clear)).toBeNull()
    expect(clear).not.toHaveBeenCalled()
  })

  it('clears an empty parameter without offering anything, and every copy of a repeated one', () => {
    const clear = vi.fn()
    expect(takeOpenLink(at('?open='), clear)).toBeNull()
    expect(clear).toHaveBeenCalledWith('/app')
    const twice = vi.fn()
    expect(takeOpenLink(at('?open=https%3A%2F%2Fa.org%2F1&open=https%3A%2F%2Fb.org%2F2'), twice)).toBe('https://a.org/1')
    expect(twice).toHaveBeenCalledWith('/app')
  })

  it('moves an old Journey link on jsDelivr to the current host, and only that', () => {
    const day = '2026-11-19-lame-de-fond/project.faulkpack'
    const raw = `https://raw.githubusercontent.com/sprkl/faulk-journey/main/tracks/${day}`
    expect(currentLink(`https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/${day}`)).toBe(raw)
    expect(takeOpenLink(at(`?open=${encodeURIComponent(`https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/${day}`)}`), () => undefined)).toBe(raw)
    const same = [
      raw,
      `https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/${day}?x=1`,
      `https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@abc1234/tracks/${day}`,
      `https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/../x/${day}`,
      `https://cdn.jsdelivr.net/gh/sprkl/other@main/tracks/${day}`,
      `https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/${day.replace('.faulkpack', '.mp3')}`,
      `http://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main/tracks/${day}`,
      'https://a.org/p.faulkpack',
    ]
    for (const link of same) expect(currentLink(link)).toBe(link)
  })

  it('moves an old link to the host the Journey page itself uses', () => {
    const e = ENTRIES[0]
    const old = packUrl(e).replace('https://raw.githubusercontent.com/sprkl/faulk-journey/main', 'https://cdn.jsdelivr.net/gh/sprkl/faulk-journey@main')
    expect(old).not.toBe(packUrl(e))
    expect(currentLink(old)).toBe(packUrl(e))
  })

  it('builds the address that opens a link, which takes it back whole', () => {
    const link = 'https://a.org/x y/p.faulkpack?v=1&w=2#f'
    const href = openLinkHref('/app', link)
    const url = new URL(href, 'https://faulk.app')
    expect(takeOpenLink({ pathname: url.pathname, search: url.search, hash: url.hash }, () => undefined)).toBe(link)
  })
})
