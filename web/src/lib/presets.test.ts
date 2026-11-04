import { describe, expect, it } from 'vitest'
import type { PresetEntry } from './generated/protocol'
import { neighbour, presetPanel, type PresetList } from './presets'

const entry = (location: 'factory' | 'project' | 'library', id: string, name: string, newer = false): PresetEntry => ({
  preset: { location, id },
  name,
  category: null,
  kind: 'faulk.osc_synth',
  version: 1,
  read_only: location === 'factory',
  newer,
  params: {},
})

const list = (over: Partial<PresetList> = {}): PresetList => ({
  presets: [entry('factory', 'f/a', 'A'), entry('project', 'P1', 'B'), entry('library', 'L1', 'C', true), entry('library', 'L2', 'D')],
  current: null,
  modified: null,
  ...over,
})

describe('the presets of a device panel', () => {
  it('leaves out what cannot be loaded and steps around the list both ways', () => {
    const none = presetPanel(list())
    expect(none.entries.map((e) => e.name)).toEqual(['A', 'B', 'D'])
    expect(neighbour(none, 1)?.name).toBe('A')
    expect(neighbour(none, -1)?.name).toBe('D')
    const onB = presetPanel(list({ current: { location: 'project', id: 'P1', name: 'B' } }))
    expect(onB.index).toBe(1)
    expect(neighbour(onB, 1)?.name).toBe('D')
    expect(neighbour(onB, -1)?.name).toBe('A')
    const onD = presetPanel(list({ current: { location: 'library', id: 'L2', name: 'D' } }))
    expect(neighbour(onD, 1)?.name).toBe('A')
  })

  it('shows the present name of a renamed preset, and the remembered one of a preset that is not here', () => {
    const renamed = presetPanel(list({ current: { location: 'project', id: 'P1', name: 'Old' }, modified: true }))
    expect(renamed.current?.name).toBe('B')
    expect(renamed.reachable).toBe(true)
    expect(renamed.modified).toBe(true)
    const gone = presetPanel(list({ current: { location: 'library', id: 'ELSEWHERE', name: 'Far' } }))
    expect(gone.current?.name).toBe('Far')
    expect(gone.reachable).toBe(false)
    expect(gone.index).toBe(-1)
  })

  it('is empty before the core exists, and has no neighbour then', () => {
    const empty = presetPanel(null)
    expect(empty.entries).toEqual([])
    expect(neighbour(empty, 1)).toBeNull()
  })
})
