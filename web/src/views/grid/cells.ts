/**
 * What the launch grid shows, as pure functions of the project and the engine's report: the
 * scenes in order, the clip of each cell, and the state of each cell. The engine reports a cell by
 * slots (its track's slot, its scene's slot); the project core gives the slots of the ids
 * (`ProjectView.slots`), so a cell's state is found by its ids.
 */

import type { ClipState, FeedbackReport } from '../../lib/engine/feedback'
import type { Clip, Project, ProjectView, Scene } from '../../lib/generated/protocol'
import { midiClips } from '../piano-roll/clips'

export interface SceneEntry {
  id: string
  scene: Scene
  /** Position in the grid, 0 first. */
  index: number
}

/** The scenes in the order the user sees: by order key, then by id (two scenes may share a key after a merge). */
export function sortedScenes(project: Pick<Project, 'scenes'>): SceneEntry[] {
  const ids = Object.keys(project.scenes).filter((id) => project.scenes[id])
  const key = (id: string) => project.scenes[id]!.order
  ids.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : a < b ? -1 : a > b ? 1 : 0))
  return ids.map((id, index) => ({ id, scene: project.scenes[id]!, index }))
}

/** The clip a cell holds. */
export interface CellClip {
  id: string
  clip: Clip
  /** Whether the engine can launch it: a MIDI clip (an audio clip is refused until AU-4). */
  launchable: boolean
}

/** Key of a cell in the maps below. */
export const cellKey = (track: string, scene: string): string => `${track}|${scene}`

/** The clip of every occupied cell: one pass over the project, read once per change of the project. */
export function cellClips(project: Pick<Project, 'tracks'>): Map<string, CellClip> {
  const cells = new Map<string, CellClip>()
  for (const [trackId, track] of Object.entries(project.tracks)) {
    if (!track) continue
    for (const [id, clip] of Object.entries(track.clips)) {
      if (!clip?.slot) continue
      cells.set(cellKey(trackId, clip.slot), { id, clip, launchable: 'midi' in clip.content })
    }
  }
  return cells
}

/** Where a clip of the grid sits. */
export interface ClipHome {
  track: string
  scene: string
  cell: CellClip
}

/** The home of every clip of the grid, by clip identifier. */
export function clipHomes(cells: ReadonlyMap<string, CellClip>): Map<string, ClipHome> {
  const homes = new Map<string, ClipHome>()
  for (const [key, cell] of cells) {
    const cut = key.indexOf('|')
    homes.set(cell.id, { track: key.slice(0, cut), scene: key.slice(cut + 1), cell })
  }
  return homes
}

/**
 * The number of each MIDI clip in its track, the one the piano roll gives it ("Clip n" when the clip
 * has no name): the same list, so the cell and the roll's title agree.
 */
export function clipNumbers(project: Pick<Project, 'tracks'>): Map<string, number> {
  const numbers = new Map<string, number>()
  for (const track of Object.values(project.tracks)) {
    if (track) midiClips(track).forEach((entry, i) => numbers.set(entry.id, i + 1))
  }
  return numbers
}

/** How many clips a scene holds, by scene. */
export function clipCounts(cells: ReadonlyMap<string, CellClip>): Map<string, number> {
  const counts = new Map<string, number>()
  for (const key of cells.keys()) {
    const scene = key.slice(key.indexOf('|') + 1)
    counts.set(scene, (counts.get(scene) ?? 0) + 1)
  }
  return counts
}

/** What the engine reports of one cell: its state, and whether a launch of the same clip waits at its boundary. */
export interface ActiveCell {
  state: ClipState
  /** A playing clip that was launched again: it restarts at the next boundary. */
  restarting: boolean
}

/**
 * The cells that are not idle: the engine's report put back in terms of ids. A report about a
 * track or scene the page has no slot for (the engine a little ahead or behind the project) is
 * ignored, and so is a report of another epoch than the slot table's (an engine behind a
 * snapshot names other slots). A cell reported twice is a clip launched again while it plays
 * (`playing`, then `queued` or `armed`, in either order): it shows `playing` and is marked as
 * restarting. The engine never reports a stopping clip and a waiting launch for one cell.
 */
export function activeCells(report: Pick<FeedbackReport, 'epoch' | 'clips'> | null, slots: ProjectView['slots']): Map<string, ActiveCell> {
  const states = new Map<string, ActiveCell>()
  if (!report || report.epoch !== slots.epoch) return states
  const tracks = new Map<number, string>()
  for (const [id, slot] of Object.entries(slots.tracks)) if (slot !== undefined) tracks.set(slot, id)
  const scenes = new Map<number, string>()
  for (const [id, slot] of Object.entries(slots.scenes)) if (slot !== undefined) scenes.set(slot, id)
  for (const { track, scene, state } of report.clips) {
    const trackId = tracks.get(track)
    const sceneId = scenes.get(scene)
    if (!trackId || !sceneId) continue
    const key = cellKey(trackId, sceneId)
    const seen = states.get(key)
    const waiting = state === 'queued' || state === 'armed'
    if (!seen) states.set(key, { state, restarting: false })
    else if (waiting && seen.state === 'playing') seen.restarting = true
    else if (!waiting && (seen.state === 'queued' || seen.state === 'armed')) states.set(key, { state, restarting: state === 'playing' })
  }
  return states
}

/**
 * The items `[first, last)` of `count` items of one size that a viewport shows, `overscan` more on
 * each side. `lead` is the room before the first item (a header). The grid draws only these: a
 * 32 by 64 grid has 2 048 cells, and the page shows about a tenth of them.
 */
export function visibleRange(offset: number, view: number, size: number, count: number, overscan: number, lead = 0): { first: number; last: number } {
  const first = Math.min(count, Math.max(0, Math.floor((offset - lead) / size) - overscan))
  const last = Math.min(count, Math.max(first, Math.ceil((offset + view - lead) / size) + overscan))
  return { first, last }
}

/**
 * A string that changes exactly when a cell changes state or the engine changes epoch: the engine
 * reports about 30 times a second, and the grid redraws only when this changes.
 */
export function activeSignature(report: Pick<FeedbackReport, 'epoch' | 'clips'> | null | undefined): string {
  return report ? `${report.epoch}|${report.clips.map((cell) => `${cell.track}.${cell.scene}.${cell.state}`).join(' ')}` : ''
}
