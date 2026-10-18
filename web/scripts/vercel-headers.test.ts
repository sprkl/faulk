import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { SITE_WIDE, siteWideHeaders } from './vercel-headers'

const isolation = [
  { key: 'Cross-Origin-Opener-Policy', value: 'same-origin' },
  { key: 'Cross-Origin-Embedder-Policy', value: 'require-corp' },
]

describe('vercel.json headers mirrored on the local servers', () => {
  it('isolates the whole site in the real vercel.json', () => {
    const config = JSON.parse(readFileSync(new URL('../vercel.json', import.meta.url), 'utf8'))
    expect(siteWideHeaders(config)).toEqual(isolation)
  })

  it('applies every site-wide entry, in order', () => {
    const headers = siteWideHeaders({
      headers: [
        { source: SITE_WIDE, headers: [{ key: 'X-One', value: '1' }] },
        { source: SITE_WIDE, headers: [{ key: 'X-Two', value: '2' }] },
      ],
    })
    expect(headers).toEqual([
      { key: 'X-One', value: '1' },
      { key: 'X-Two', value: '2' },
    ])
  })

  it('has nothing to set without a "headers" entry', () => {
    expect(siteWideHeaders({ cleanUrls: true })).toEqual([])
  })

  it.each([
    ['an exact path', { source: '/app', headers: isolation }],
    ['another pattern', { source: '/app/(.*)', headers: isolation }],
    ['a named parameter', { source: '/:path*', headers: isolation }],
    ['a host condition', { source: SITE_WIDE, has: [{ type: 'host', value: 'x' }], headers: isolation }],
    ['a "missing" condition', { source: SITE_WIDE, missing: [{ type: 'host', value: 'x' }], headers: isolation }],
    ['no header', { source: SITE_WIDE, headers: [] }],
    ['a header without value', { source: SITE_WIDE, headers: [{ key: 'X' }] }],
  ])('refuses %s', (_, entry) => {
    expect(() => siteWideHeaders({ headers: [entry] })).toThrow(/vercel\.json: headers\[0\]/)
  })

  it('refuses a "headers" field that is not a list', () => {
    expect(() => siteWideHeaders({ headers: {} })).toThrow(/must be an array/)
  })
})
