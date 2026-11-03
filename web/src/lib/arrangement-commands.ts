import type { Command, Tick } from './generated/protocol'

/**
 * The commands of the arrangement view: every gesture of the timeline is one of them, sent once when the
 * gesture ends (a drag is one command, so one history entry and one undo). They are the same commands
 * an agent calls (`crates/commands/src/arrangement.rs`).
 */

/** Moves placements in time, all by the same number of ticks (negative is earlier). */
export const movePlacements = (placements: string[], by: Tick): Command => ({ type: 'arrangement.move', placements, by })
/** Moves one edge of a placement to a timeline tick. */
export const resizePlacement = (placement: string, edge: 'start' | 'end', to: Tick): Command => ({ type: 'arrangement.resize', placement, edge, to })
/** Splits a placement in two at a timeline tick strictly inside it. */
export const splitPlacement = (placement: string, at: Tick): Command => ({ type: 'arrangement.split', placement, at })
export const deletePlacements = (placements: string[]): Command => ({ type: 'arrangement.delete', placements })
/** Copies placements `by` ticks later: independent clips by default, the same clips (linked) with `linked`. */
export const duplicatePlacements = (placements: string[], linked: boolean, by?: Tick): Command =>
  by === undefined ? { type: 'arrangement.duplicate', placements, linked } : { type: 'arrangement.duplicate', placements, linked, by }
/** Puts a clip of the launch grid in the arrangement from a tick: an independent copy by default, the grid clip itself (linked) with `linked`. */
export const placeFromSlot = (clip: string, start: Tick, linked: boolean): Command => ({ type: 'arrangement.place_from_slot', clip, start, linked })
/** The same for every clip of a scene, each on its own track: one command, one history entry. */
export const placeScene = (scene: string, start: Tick, linked: boolean): Command => ({ type: 'arrangement.place_scene', scene, start, linked })
/** Opens `length` ticks of empty time at a tick, in every track and the master (#396): one command, one history entry. */
export const insertTime = (at: Tick, length: Tick): Command => ({ type: 'arrangement.insert_time', at, length })
/** Deletes the time `[from, to)` from every track and the master and closes the gap (#396). */
export const deleteTime = (from: Tick, to: Tick): Command => ({ type: 'arrangement.delete_time', from, to })
/** Section markers (#473): add one (a name, or the next "Section n"), rename, move, delete. */
export const addMarker = (tick: Tick, name?: string): Command =>
  name === undefined ? { type: 'arrangement.marker.add', tick } : { type: 'arrangement.marker.add', tick, name }
export const renameMarker = (marker: string, name: string): Command => ({ type: 'arrangement.marker.rename', marker, name })
export const moveMarker = (marker: string, tick: Tick): Command => ({ type: 'arrangement.marker.move', marker, tick })
export const deleteMarker = (marker: string): Command => ({ type: 'arrangement.marker.delete', marker })
/** The start of the bar a tick lies in: where a clip sent from the grid is put. */
export const barStart = (tick: number, bar: number): number => (bar > 0 ? Math.floor(Math.max(0, tick) / bar) * bar : 0)
/** A one-bar clip put in the arrangement of a track, from a tick. */
export const createClipAt = (track: string, start: Tick, length: Tick): Command => ({ type: 'clip.create', track, at: { at: 'arrangement', start }, length })
