import type { Command, PresetRef } from './generated/protocol'

/** The commands of a device's presets (#191): the header of its panel sends them, as an agent would. */
export const loadPreset = (device: string, preset: PresetRef): Command => ({ type: 'preset.load', device, preset })
export const savePreset = (device: string, name: string, location: 'project' | 'library'): Command => ({ type: 'preset.save', device, name, location })
export const renamePreset = (preset: PresetRef, name: string): Command => ({
  type: 'preset.rename',
  preset: { location: preset.location, id: preset.id },
  name,
})
export const copyPreset = (preset: PresetRef, location: 'project' | 'library'): Command => ({
  type: 'preset.copy',
  preset: { location: preset.location, id: preset.id },
  location,
})
export const deletePreset = (preset: PresetRef): Command => ({ type: 'preset.delete', preset: { location: preset.location, id: preset.id } })
/** The presets of a device, or of a device kind (`preset.list`): a query, answered at once. */
export const listPresets = (target: { device: string } | { kind: string }): Command => ({ type: 'preset.list', ...target })
