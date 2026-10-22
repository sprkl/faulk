import type { Command } from './generated/protocol'

/**
 * The commands of the launch grid: scenes, launches and the clip a cell creates. The launches are
 * live (no undo, no project change); the scene operations are edits, each one history entry.
 */

export const createScene = (): Command => ({ type: 'scene.create' })
export const deleteScene = (scene: string): Command => ({ type: 'scene.delete', scene })
export const renameScene = (scene: string, name: string): Command => ({ type: 'scene.rename', scene, name })
export const moveScene = (scene: string, index: number): Command => ({ type: 'scene.move', scene, index })

export const launchClip = (clip: string): Command => ({ type: 'launch.clip', clip })
export const launchScene = (scene: string): Command => ({ type: 'launch.scene', scene })
export const stopTrack = (track: string): Command => ({ type: 'launch.stop_track', track })
export const stopAll = (): Command => ({ type: 'launch.stop_all' })

/** A one-bar MIDI clip in the cell of `track` and `scene` (the project's default length). */
export const createClipInCell = (track: string, scene: string): Command => ({ type: 'clip.create', track, at: { at: 'slot', scene } })
