import { describe, expect, it } from 'vitest'
import type { PresetEntry } from '../generated/protocol'
import { DEVICES } from '../generated/protocol'
import type { DeviceDescriptor } from '../devices'
import { buildCatalog, filterCatalog, normalize, rowsOf, type BrowserGroup } from './catalog'

const entry = (kind: string, name: string, location: 'factory' | 'project' | 'library' = 'factory', over: Partial<PresetEntry> = {}): PresetEntry => ({
  preset: { location, id: `${kind}/${name}` },
  name,
  category: null,
  kind,
  version: 1,
  read_only: location === 'factory',
  newer: false,
  params: {},
  ...over,
})

const PRESETS: Record<string, PresetEntry[]> = {
  'faulk.osc_synth': [entry('faulk.osc_synth', 'Round Bass'), entry('faulk.osc_synth', 'Soft Pad'), entry('faulk.osc_synth', 'Café Lead', 'library')],
  'faulk.reverb': [entry('faulk.reverb', 'Large Hall'), entry('faulk.reverb', 'Small Room'), entry('faulk.reverb', 'Too New', 'library', { newer: true })],
}
const catalog = () => buildCatalog((kind) => PRESETS[kind] ?? [])
const names = (groups: readonly BrowserGroup[]) => groups.flatMap((g) => g.devices.map((d) => d.name))

describe('the browser list', () => {
  it('lists every device of the registry, each once, grouped by role, with no name written here', () => {
    const groups = catalog()
    const listed = groups.flatMap((g) => g.devices.map((d) => d.kind))
    expect(listed.sort()).toEqual(DEVICES.map((d) => d.kind).sort())
    for (const group of groups) {
      for (const device of group.devices)
        expect(DEVICES.find((d) => d.kind === device.kind)?.role === 'instrument' ? 'instrument' : group.category).toBe(group.category)
    }
    expect(groups.map((g) => g.category)).toEqual(['instrument', 'effect'])
  })

  it('shows a device added to the registry, in its group, with no change to the list', () => {
    const extra = [
      ...DEVICES,
      { ...DEVICES[0], kind: 'acme.pluck', name: 'Pluck' },
      { ...DEVICES[2], kind: 'acme.gate', name: 'Gate', role: 'midi_effect' },
    ] as unknown as readonly DeviceDescriptor[]
    const groups = buildCatalog(() => [], extra)
    expect(groups.map((g) => g.category)).toEqual(['instrument', 'effect', 'midi_effect'])
    expect(groups[0]!.devices.map((d) => d.name)).toContain('Pluck')
    expect(groups[2]!.devices.map((d) => d.name)).toEqual(['Gate'])
  })

  it('lists the presets that can be loaded under their device, factory and user, and leaves out a newer one', () => {
    const synth = catalog()[0]!.devices.find((d) => d.kind === 'faulk.osc_synth')!
    expect(synth.presets.map((p) => [p.name, p.ref.location])).toEqual([
      ['Round Bass', 'factory'],
      ['Soft Pad', 'factory'],
      ['Café Lead', 'library'],
    ])
    const reverb = catalog()[1]!.devices.find((d) => d.kind === 'faulk.reverb')!
    expect(reverb.presets.map((p) => p.name)).toEqual(['Large Hall', 'Small Room'])
  })

  it('walks device then presets, in display order, with a stable id per row', () => {
    const rows = rowsOf(catalog())
    expect(new Set(rows.map((r) => r.id)).size).toBe(rows.length)
    const at = rows.findIndex((r) => r.id === 'faulk.osc_synth')
    expect(rows[at + 1]!.item.preset?.name).toBe('Round Bass')
    expect(rows[at + 1]!.item.kind).toBe('faulk.osc_synth')
  })
})

describe('the search', () => {
  it('normalizes case, accents and spaces', () => {
    expect(normalize('  ÉCHO   Écho ')).toBe('echo echo')
    expect(normalize('Café')).toBe('cafe')
  })

  it('finds by device name, whatever the case', () => {
    expect(names(filterCatalog(catalog(), 'REVERB'))).toEqual(['Reverb'])
    expect(names(filterCatalog(catalog(), 'osc'))).toEqual(['Oscillator'])
  })

  it('keeps all the presets of a device whose name matches', () => {
    const reverb = filterCatalog(catalog(), 'reverb')[0]!.devices[0]!
    expect(reverb.presets.map((p) => p.name)).toEqual(['Large Hall', 'Small Room'])
  })

  it('finds by preset name and keeps only the presets that match, under their device', () => {
    const groups = filterCatalog(catalog(), 'hall')
    expect(groups.map((g) => g.category)).toEqual(['effect'])
    expect(groups[0]!.devices.map((d) => [d.name, d.presets.map((p) => p.name)])).toEqual([['Reverb', ['Large Hall']]])
  })

  it('ignores accents on both sides', () => {
    expect(filterCatalog(catalog(), 'cafe')[0]!.devices[0]!.presets.map((p) => p.name)).toEqual(['Café Lead'])
    expect(filterCatalog(catalog(), 'CAFÉ')[0]!.devices[0]!.presets.map((p) => p.name)).toEqual(['Café Lead'])
  })

  it('takes several words, split between the device and the preset', () => {
    const groups = filterCatalog(catalog(), 'reverb small')
    expect(groups[0]!.devices[0]!.presets.map((p) => p.name)).toEqual(['Small Room'])
    expect(filterCatalog(catalog(), 'reverb bass')).toEqual([])
  })

  it('has no result for a text nothing matches, and gives the whole list back when it is cleared', () => {
    expect(filterCatalog(catalog(), 'zzzz')).toEqual([])
    expect(rowsOf(filterCatalog(catalog(), '   '))).toEqual(rowsOf(catalog()))
    expect(rowsOf(filterCatalog(catalog(), ''))).toEqual(rowsOf(catalog()))
  })

  it('does not match a preset the list leaves out (newer than this Faulk)', () => {
    expect(filterCatalog(catalog(), 'too new')).toEqual([])
  })
})
