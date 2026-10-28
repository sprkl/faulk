import type { AutomationTarget, Command, Point, Tick } from './generated/protocol'

/**
 * The commands of the automation lanes (AM-1): every gesture of the lane editor in the timeline is one of
 * them (`crates/commands/src/automation.rs`), sent once when the gesture ends. A drag that moves points in
 * time is several of them inside one history group (`track-commands.ts`: `beginGroup`, `endGroup`).
 */

/** A lane for a parameter of a track: a device's, or the track's volume or pan. One lane per parameter and track. */
export const createLane = (track: string, target: AutomationTarget): Command => ({ type: 'automation.lane.create', track, target })
export const deleteLane = (lane: string): Command => ({ type: 'automation.lane.delete', lane })
/** New points (a tick that already holds a point is refused). */
export const addPoints = (lane: string, points: Point[]): Command => ({ type: 'automation.points.add', lane, points })
export const removePoints = (lane: string, ticks: Tick[]): Command => ({ type: 'automation.points.remove', lane, ticks })
/** Changes the value or the curve of points that exist (a tick without a point is refused). */
export const updatePoints = (lane: string, points: Point[]): Command => ({ type: 'automation.points.update', lane, points })
export const clearLane = (lane: string): Command => ({ type: 'automation.lane.clear', lane })
