import type { Command } from './generated/protocol'

/**
 * The commands of the device panel (DV-8): each action of a device card or of the browser is one
 * of them. A track's chain is its instrument, always first, then its effects; `index` counts the
 * effects only, 0 right after the instrument.
 */
export const insertEffect = (track: string, kind: string, index: number): Command => ({ type: 'device.insert', track, kind, index })
/** Inserts an effect in the master's chain (`master: true` names it; the master is not a track). */
export const insertMasterEffect = (kind: string, index: number): Command => ({ type: 'device.insert', master: true, kind, index })
export const removeDevice = (device: string): Command => ({ type: 'device.remove', device })
export const moveDevice = (device: string, index: number): Command => ({ type: 'device.move', device, index })
export const setBypass = (device: string, bypass: boolean): Command => ({ type: 'device.set_bypass', device, bypass })
export const setInstrument = (track: string, instrument: string): Command => ({ type: 'track.set_instrument', track, instrument })
export const setDeviceParam = (device: string, param: string, value: number): Command => ({ type: 'device.set_param', device, param, value })
