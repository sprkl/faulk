import type { Command, TrackKind } from './generated/protocol'

/**
 * The commands of the track list: each action of a track header is one of them. Colours and
 * names are the user's; a track created without a name shows a default name in the page's
 * language (`TrackHeader`), never one stored. Without `color`, the command stores the next free hue of
 * the palette with the track (the same for the UI and for an agent), so a colour never follows the order.
 */
export const createTrack = (kind: TrackKind, instrument: string | null, color?: string): Command =>
  kind === 'midi' ? { type: 'track.create', kind, instrument, color } : { type: 'track.create', kind, color }

export const deleteTrack = (track: string): Command => ({ type: 'track.delete', track })
/** Puts a track at `index` in the list, 0 first: one command, one history entry, whatever the distance. */
export const moveTrack = (track: string, index: number): Command => ({ type: 'track.move', track, index })
/** Copies a track right below the source; `name` is the copy's (the command keeps the source's name without it). */
export const duplicateTrack = (track: string, name: string): Command => ({ type: 'track.duplicate', track, name })
export const renameTrack = (track: string, name: string): Command => ({ type: 'track.rename', track, name })
export const setTrackColor = (track: string, color: string): Command => ({ type: 'track.set_color', track, color })
export const selectTrack = (track: string): Command => ({ type: 'view.select_track', track })
export const setVolume = (track: string, gainDb: number | null): Command => ({ type: 'mixer.set_volume', track, gain_db: gainDb })
export const setPan = (track: string, pan: number): Command => ({ type: 'mixer.set_pan', track, pan })
export const setMute = (track: string, mute: boolean): Command => ({ type: 'mixer.set_mute', track, mute })
export const setSolo = (track: string, solo: boolean): Command => ({ type: 'mixer.set_solo', track, solo })
/** Arming and monitoring are session state: live input reaches the armed tracks, never undone. */
export const armTrack = (track: string, armed: boolean): Command => ({ type: 'track.arm', track, armed })
export const setMonitor = (track: string, monitor: boolean): Command => ({ type: 'track.set_monitor', track, monitor })
export const undo = (): Command => ({ type: 'history.undo' })
export const redo = (): Command => ({ type: 'history.redo' })
/** A fader or pan gesture is one history group: one undo takes the whole drag back. */
export const beginGroup = (): Command => ({ type: 'history.begin_group' })
export const endGroup = (): Command => ({ type: 'history.end_group' })
/** The master's volume in dB (`null` is silence). There is no master mute command. */
export const setMasterVolume = (gainDb: number | null): Command => ({ type: 'mixer.set_master_volume', gain_db: gainDb })
/** The master's pan, from -1 (left) to 1 (right). */
export const setMasterPan = (pan: number): Command => ({ type: 'mixer.set_master_pan', pan })
