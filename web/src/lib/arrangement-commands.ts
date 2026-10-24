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
/** A one-bar clip put in the arrangement of a track, from a tick. */
export const createClipAt = (track: string, start: Tick, length: Tick): Command => ({ type: 'clip.create', track, at: { at: 'arrangement', start }, length })
