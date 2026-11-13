// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { descriptorOf } from '../devices'
import { cardGestures, controlGroups, curveRate } from './card'

const types = (run: ReturnType<typeof vi.fn>) => run.mock.calls.map(([command]) => command.type)

describe("Contour's card", () => {
  it('warps the curve at the engine rate first, then the last spectrum rate, then 48 kHz', () => {
    expect(curveRate(96_000, 48_000)).toBe(96_000)
    expect(curveRate(0, 44_100)).toBe(44_100)
    expect(curveRate(undefined, 44_100)).toBe(44_100)
    expect(curveRate(null, undefined)).toBe(48_000)
  })

  it('ends the curve group when a knob begins, so the curve wheel timer never cuts a knob drag', () => {
    // A refused `begin_group` (a group is open) is what the history does: the run says false.
    let open = false
    const run = vi.fn((command: { type: string }) => {
      if (command.type === 'history.begin_group') {
        if (open) return false
        open = true
      }
      if (command.type === 'history.end_group') open = false
      return true
    })
    const g = cardGestures(run)
    // The wheel opens the curve's group, then a knob drag starts before the wheel's rest.
    g.curve.begin()
    g.knobs.begin()
    expect(types(run)).toEqual(['history.begin_group', 'history.end_group', 'history.begin_group'])
    // The wheel's rest comes: it ends only the curve's group, which is already closed.
    g.curve.end()
    expect(open).toBe(true)
    g.knobs.end()
    expect(open).toBe(false)
    // The other way round: a curve gesture during a knob's ends the knob's first.
    run.mockClear()
    g.knobs.begin()
    g.curve.begin()
    g.end()
    expect(types(run)).toEqual(['history.begin_group', 'history.end_group', 'history.begin_group', 'history.end_group'])
  })
})

describe("Contour's controls by band (#855)", () => {
  const params = descriptorOf('faulk.contour').params

  it('puts every parameter in exactly one group, the bands in the order of the signal and the output last', () => {
    const groups = controlGroups(params)
    expect(groups.map((g) => g.key)).toEqual(['hp', 'low', 'bell1', 'bell2', 'bell3', 'high', 'lp', 'output'])
    expect(groups.map((g) => g.number)).toEqual([1, 2, 3, 4, 5, 6, 7, null])
    const placed = groups.flatMap((g) => [...(g.on ? [g.on.key] : []), ...g.params.map((p) => p.key)])
    expect(placed.toSorted()).toEqual(params.map((p) => p.key).toSorted())
    expect(new Set(placed).size).toBe(params.length)
  })

  it('takes a band switch apart from its knobs, and gives the output no switch', () => {
    const groups = controlGroups(params)
    for (const g of groups.slice(0, 7)) {
      expect(g.on?.key).toBe(`${g.key}_on`)
      expect(g.params.every((p) => p.key.startsWith(`${g.key}_`) && p.key !== `${g.key}_on`)).toBe(true)
    }
    expect(groups[0].params.map((p) => p.key)).toEqual(['hp_freq', 'hp_q', 'hp_slope'])
    expect(groups[7].on).toBeNull()
    expect(groups[7].params.map((p) => p.key)).toEqual(['output'])
  })

  it('leaves out a band whose parameters are all hidden', () => {
    const groups = controlGroups(params.filter((p) => !p.key.startsWith('bell2_')))
    expect(groups.map((g) => g.key)).not.toContain('bell2')
  })
})
