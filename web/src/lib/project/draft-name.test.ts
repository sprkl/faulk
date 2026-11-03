import { describe, expect, it } from 'vitest'
import { draftName } from './draft-name'

describe('draftName (#473)', () => {
  const at = new Date(2026, 10, 2, 14, 5)

  it('names a draft after the moment it started, in the page language', () => {
    expect(draftName('en', at)).toMatch(/^Sketch Nov 2, 2\.05\sPM$/)
    expect(draftName('fr', at)).toBe('Esquisse 2 nov., 14h05')
  })

  it('has no character a file name refuses', () => {
    for (const lang of ['en', 'fr'] as const) expect(draftName(lang, at)).not.toMatch(/[\\/:*?"<>|]/)
  })
})
