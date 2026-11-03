import { describe, expect, it } from 'vitest'
import { checkoutHelperPort, e2eHelperPort, E2E_HELPER_PORT_COUNT, E2E_HELPER_PORT_FIRST } from './e2e-helper-port'

describe('the e2e helper port of a checkout (#505)', () => {
  it('is stable for a path, whatever its separators, case and trailing slash', () => {
    const a = checkoutHelperPort('C:\\git\\faulk\\faulk\\web\\')
    expect(checkoutHelperPort('c:/git/faulk/faulk/web')).toBe(a)
    expect(checkoutHelperPort('C:/git/faulk/faulk/web/')).toBe(a)
  })

  it('stays in its range, never on the installed helper (47800)', () => {
    for (let i = 0; i < 5000; i++) {
      const port = checkoutHelperPort(`/work/checkout-${i}/web`)
      expect(port).toBeGreaterThanOrEqual(E2E_HELPER_PORT_FIRST)
      expect(port).toBeLessThan(E2E_HELPER_PORT_FIRST + E2E_HELPER_PORT_COUNT)
    }
    expect(E2E_HELPER_PORT_FIRST).toBeGreaterThan(47800)
    expect(E2E_HELPER_PORT_FIRST + E2E_HELPER_PORT_COUNT - 1).toBeLessThanOrEqual(47999)
  })

  it('spreads checkouts over the whole range: 1000 worktree paths use nearly every port, none takes more than a few percent', () => {
    const uses = new Map<number, number>()
    for (let i = 0; i < 1000; i++) {
      const port = checkoutHelperPort(`C:/git/faulk/faulk/.claude/worktrees/agent-${i.toString(16).padStart(8, '0')}/web`)
      uses.set(port, (uses.get(port) ?? 0) + 1)
    }
    // 1000 draws over 190 ports leave about 189 used (a 150 floor leaves room); the busiest holds about 13 draws (30 is far off a fair spread).
    expect(uses.size).toBeGreaterThanOrEqual(150)
    expect(Math.max(...uses.values())).toBeLessThanOrEqual(30)
  })

  it('takes FAULK_E2E_HELPER_PORT first, and refuses what is no port', () => {
    expect(e2eHelperPort({ FAULK_E2E_HELPER_PORT: '47400' }, '/a/web')).toBe(47400)
    expect(e2eHelperPort({ FAULK_E2E_HELPER_PORT: ' 47400 ' }, '/a/web')).toBe(47400)
    for (const bad of ['abc', '0', '65536', '47.5', '-1']) expect(() => e2eHelperPort({ FAULK_E2E_HELPER_PORT: bad }, '/a/web'), bad).toThrow(/not a port/)
  })

  it('falls back to the checkout port when the variable is unset or blank', () => {
    const own = checkoutHelperPort('/a/web')
    expect(e2eHelperPort({}, '/a/web')).toBe(own)
    expect(e2eHelperPort({ FAULK_E2E_HELPER_PORT: '' }, '/a/web')).toBe(own)
    expect(e2eHelperPort({ FAULK_E2E_HELPER_PORT: '  ' }, '/a/web')).toBe(own)
  })

  it('answers for this checkout without arguments', () => {
    const port = e2eHelperPort({})
    expect(port).toBeGreaterThanOrEqual(E2E_HELPER_PORT_FIRST)
    expect(e2eHelperPort({})).toBe(port)
  })
})
