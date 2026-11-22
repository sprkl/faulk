import { describe, expect, it } from 'vitest'
import { DEVICES } from './generated/protocol'
import { gateState, SLUICE_STATE_READOUT } from './sluice'

describe('Sluice state readout (#158)', () => {
  it('decodes open, closed and their reduction', () => {
    expect(gateState(0)).toEqual({ open: true, reductionDb: 0 })
    expect(gateState(3.5)).toEqual({ open: true, reductionDb: 3.5 })
    expect(gateState(-1)).toEqual({ open: false, reductionDb: 0 })
    expect(gateState(-31)).toEqual({ open: false, reductionDb: 30 })
  })

  it('reports nothing for no report or a corrupt value', () => {
    expect(gateState(undefined)).toBeNull()
    expect(gateState(Number.NaN)).toBeNull()
    expect(gateState(Number.POSITIVE_INFINITY)).toBeNull()
  })

  it('uses an id that is not one of the descriptor’s parameters', () => {
    const sluice = DEVICES.find((d) => d.kind === 'faulk.sluice')
    expect(sluice?.name).toBe('Sluice')
    expect(sluice?.sidechain).toBe(true)
    expect(sluice?.params.map((p) => p.id as number)).not.toContain(SLUICE_STATE_READOUT)
  })
})
