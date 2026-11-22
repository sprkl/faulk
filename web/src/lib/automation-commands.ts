import type { AutomationTarget, Command, Point, Tick } from './generated/protocol'

/**
 * The commands of the automation lanes (AM-1): every gesture of the lane editor in the timeline is one of
 * them (`crates/commands/src/automation.rs`), sent once when the gesture ends. A drag that moves points in
 * time is several of them inside one history group (`track-commands.ts`: `beginGroup`, `endGroup`).
 */

/** A lane for a parameter of a track: a device's, or the track's volume or pan. One lane per parameter and track. */
export const createLane = (track: string, target: AutomationTarget): Command => ({ type: 'automation.lane.create', track, target })
/** Creates a lane on the master (`master: true` names it; the master is not a track): its volume or pan, or a parameter of an effect of its chain (#378). */
export const createMasterLane = (target: AutomationTarget): Command => ({ type: 'automation.lane.create', master: true, target })
export const deleteLane = (lane: string): Command => ({ type: 'automation.lane.delete', lane })
/** New points (a tick that already holds a point is refused). */
export const addPoints = (lane: string, points: Point[]): Command => ({ type: 'automation.points.add', lane, points })
export const removePoints = (lane: string, ticks: Tick[]): Command => ({ type: 'automation.points.remove', lane, ticks })
/** Changes the value or the curve of points that exist (a tick without a point is refused). */
export const updatePoints = (lane: string, points: Point[]): Command => ({ type: 'automation.points.update', lane, points })
export const clearLane = (lane: string): Command => ({ type: 'automation.lane.clear', lane })
/**
 * Writes points exactly as given (a point on a tick that holds one replaces it) and takes the points at `remove` out, as one
 * history entry: the inverse of the edits above, and the one command of a ramp (#769). Values are kept as they are, so the page
 * rounds them to the parameter's step first (`Axis.clamp`).
 */
export const restorePoints = (lane: string, points: Point[], remove: Tick[] = []): Command => ({ type: 'automation.points.restore', lane, points, remove })
