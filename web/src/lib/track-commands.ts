import type { Command, TrackKind } from './generated/protocol'

/**
 * The commands of the track list: each action of a track header is one of them. Colours and
 * names are the user's; a track created without a name shows a default name in the page's
 * language (`TrackHeader`), never one stored.
 */
export const createTrack = (kind: TrackKind, instrument: string | null, color: string): Command =>
  kind === 'midi'
    ? { type: 'track.create', kind, instrument, color }
    : { type: 'track.create', kind, color }

export const deleteTrack = (track: string): Command => ({ type: 'track.delete', track })
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
/** The master's volume in dB (`null` is silence). There is no master pan or master mute command. */
export const setMasterVolume = (gainDb: number | null): Command => ({ type: 'mixer.set_master_volume', gain_db: gainDb })
