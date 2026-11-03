import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Command } from '../../lib/generated/protocol'
import { setTempo } from '../../lib/transport-commands'
import { createSeries } from './series'

const types = (run: ReturnType<typeof vi.fn>) => run.mock.calls.map(([command]) => (command as Command).type)

describe('a series of tempo changes', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('is one group however many changes it holds, closed after the idle time', () => {
    const run = vi.fn(() => true)
    const series = createSeries(run, 2000)
    series.apply(setTempo(100))
    vi.advanceTimersByTime(1500)
    series.apply(setTempo(101))
    vi.advanceTimersByTime(1500)
    series.apply(setTempo(102))
    expect(series.open).toBe(true)
    expect(types(run)).toEqual(['history.begin_group', 'transport.set_tempo', 'transport.set_tempo', 'transport.set_tempo'])
    vi.advanceTimersByTime(2000)
    expect(series.open).toBe(false)
    expect(types(run).at(-1)).toBe('history.end_group')
    expect(types(run).filter((t) => t === 'history.begin_group')).toHaveLength(1)
  })

  it('starts a new group for a change after the series closed', () => {
    const run = vi.fn(() => true)
    const series = createSeries(run, 600)
    series.apply(setTempo(100))
    vi.advanceTimersByTime(600)
    series.apply(setTempo(110))
    expect(types(run).filter((t) => t === 'history.begin_group')).toHaveLength(2)
    expect(types(run).filter((t) => t === 'history.end_group')).toHaveLength(1)
  })

  it('end closes it at once, once, and cancels the timer', () => {
    const run = vi.fn(() => true)
    const series = createSeries(run, 2000)
    series.end()
    expect(run).not.toHaveBeenCalled()
    series.apply(setTempo(100))
    series.end()
    series.end()
    vi.advanceTimersByTime(5000)
    expect(types(run)).toEqual(['history.begin_group', 'transport.set_tempo', 'history.end_group'])
  })

  it('runs the change on its own when a group cannot be opened, and never closes a group it did not open', () => {
    const run = vi.fn((command: Command) => command.type !== 'history.begin_group')
    const series = createSeries(run, 2000)
    series.apply(setTempo(100))
    expect(series.open).toBe(false)
    vi.advanceTimersByTime(5000)
    series.end()
    expect(types(run)).toEqual(['history.begin_group', 'transport.set_tempo'])
  })
})
