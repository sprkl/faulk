/**
 * The device browser's list (#190): every device of the registry (`DEVICES`) with its presets, grouped by what it is
 * (instrument, audio effect, MIDI effect), and the live search over it. Nothing here names a device: a device added
 * to the registry is listed with no change.
 */

import type { PresetEntry, PresetRef } from '../generated/protocol'
import { DEVICES } from '../generated/protocol'
import type { DeviceDescriptor } from '../devices'

/** The groups of the list, in the order they are shown. */
export const CATEGORIES = ['instrument', 'effect', 'midi_effect'] as const
export type Category = (typeof CATEGORIES)[number]

/** One preset under its device. */
export interface BrowserPreset {
  ref: PresetRef
  name: string
  /** The preset's own category, when it has one. */
  category: string | null
}

/** One device with the presets it can load. */
export interface BrowserDevice {
  kind: string
  name: string
  category: Category
  presets: BrowserPreset[]
}

export interface BrowserGroup {
  category: Category
  devices: BrowserDevice[]
}

/** What is dragged, double-clicked or entered: a device, or one of its presets. */
export interface BrowserItem {
  kind: string
  name: string
  category: Category
  /** `null` for the device itself, else the preset to load once it is there. */
  preset: BrowserPreset | null
}

/** The presets of a device kind that can be loaded, as `Dispatcher.listPresetsOfKind` gives them (factory, project, library). */
export type PresetsOfKind = (kind: string) => readonly PresetEntry[]

/** The list's group of a descriptor: the role the registry declares, or `effect` for a role this page does not know yet. */
export const categoryOf = (descriptor: Pick<DeviceDescriptor, 'role'>): Category => {
  const role: string = descriptor.role
  return role === 'instrument' || role === 'midi_effect' ? role : 'effect'
}

/** The whole list, from the registry's order; a group with no device is left out. */
export function buildCatalog(presetsOf: PresetsOfKind, descriptors: readonly DeviceDescriptor[] = DEVICES): BrowserGroup[] {
  const groups: BrowserGroup[] = CATEGORIES.map((category) => ({ category, devices: [] }))
  for (const descriptor of descriptors) {
    const category = categoryOf(descriptor)
    const presets = presetsOf(descriptor.kind)
      .filter((entry) => !entry.newer)
      .map((entry): BrowserPreset => ({ ref: { location: entry.preset.location, id: entry.preset.id }, name: entry.name, category: entry.category ?? null }))
    groups.find((g) => g.category === category)!.devices.push({ kind: descriptor.kind, name: descriptor.name, category, presets })
  }
  return groups.filter((g) => g.devices.length > 0)
}

/** A text as the search compares it: lower case, without accents, spaces collapsed. "Éco" finds "echo", "ECHO" finds "Écho". */
export const normalize = (text: string): string =>
  text
    .normalize('NFD')
    .replace(/\p{M}+/gu, '')
    .toLowerCase()
    .replace(/\s+/g, ' ')
    .trim()

/**
 * The list the query keeps. Each word of the query must be found in the name of a device or of a preset: a device whose
 * own name matches keeps all its presets; otherwise it stays with the presets that match (a word may be in the device's
 * name and the others in the preset's: "reverb hall"). No query keeps everything, and no match leaves no group.
 */
export function filterCatalog(groups: readonly BrowserGroup[], query: string): BrowserGroup[] {
  const words = normalize(query).split(' ').filter(Boolean)
  if (words.length === 0) return groups.map((g) => ({ category: g.category, devices: g.devices }))
  const out: BrowserGroup[] = []
  for (const group of groups) {
    const devices: BrowserDevice[] = []
    for (const device of group.devices) {
      const deviceName = normalize(device.name)
      if (words.every((word) => deviceName.includes(word))) {
        devices.push(device)
        continue
      }
      const presets = device.presets.filter((preset) => {
        const text = `${deviceName} ${normalize(preset.name)}`
        return words.every((word) => text.includes(word))
      })
      if (presets.length > 0) devices.push({ ...device, presets })
    }
    if (devices.length > 0) out.push({ category: group.category, devices })
  }
  return out
}

/** One row of the list as the keyboard walks it: a device, or one of its presets. */
export interface BrowserRow {
  /** Stable within a list: the device kind, then the preset's location and id. */
  id: string
  item: BrowserItem
}

export const presetRowId = (kind: string, ref: PresetRef): string => `${kind}|${ref.location}|${ref.id}`

/** The list flattened in display order: each device, then its presets. The arrows of the search field walk this. */
export function rowsOf(groups: readonly BrowserGroup[]): BrowserRow[] {
  const rows: BrowserRow[] = []
  for (const group of groups) {
    for (const device of group.devices) {
      rows.push({ id: device.kind, item: { kind: device.kind, name: device.name, category: device.category, preset: null } })
      for (const preset of device.presets) {
        rows.push({ id: presetRowId(device.kind, preset.ref), item: { kind: device.kind, name: device.name, category: device.category, preset } })
      }
    }
  }
  return rows
}
