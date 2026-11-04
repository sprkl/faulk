import type { DevicePreset, PresetEntry, PresetRef } from './generated/protocol'

/** What `preset.list { device }` answers, as the dispatcher hands it over. */
export interface PresetList {
  presets: PresetEntry[]
  current: DevicePreset | null
  modified: boolean | null
}

/** What the header of a device's panel shows (#191). */
export interface PresetPanel {
  /** The presets of the device's kind that it can load, in the list's order (factory, project, library; each by name). */
  entries: PresetEntry[]
  /** The preset the device shows, with its present name when it is still here; `null` for none. */
  current: (PresetRef & { name: string }) | null
  /** Whether the shown preset is here (a library preset of another computer is not). */
  reachable: boolean
  /** `true` when the settings differ from the shown preset; `null` when that cannot be told. */
  modified: boolean | null
  /** The index of the shown preset in `entries`, or -1. */
  index: number
}

const same = (a: PresetRef, b: PresetRef) => a.location === b.location && a.id === b.id

export function presetPanel(list: PresetList | null): PresetPanel {
  if (!list) return { entries: [], current: null, reachable: false, modified: null, index: -1 }
  const entries = list.presets.filter((e) => !e.newer)
  const shown = list.current
  const index = shown ? entries.findIndex((e) => same(e.preset, shown)) : -1
  const found = index >= 0 ? entries[index] : undefined
  return {
    entries,
    // A renamed preset shows its new name; one that is gone keeps the name the device remembers.
    current: shown ? { location: shown.location, id: shown.id, name: found?.name ?? shown.name } : null,
    reachable: found !== undefined,
    modified: list.modified,
    index,
  }
}

/** The preset before (`-1`) or after (`1`) the shown one, wrapping around; the first or the last when none is shown. */
export function neighbour(panel: PresetPanel, direction: -1 | 1): PresetEntry | null {
  const n = panel.entries.length
  if (n === 0) return null
  if (panel.index < 0) return panel.entries[direction === 1 ? 0 : n - 1]!
  return panel.entries[(panel.index + direction + n) % n]!
}
