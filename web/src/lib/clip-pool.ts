/**
 * The clip pool (#473): every clip of a track, wherever it is used (its placements in the arrangement, its launch-grid
 * slot) or nowhere (a clip that lost its last placement stays in its track). The Clips panel lists it; a clip of it
 * goes to the arrangement or to the grid by a drag or a button. `linked` picks the command: false makes an independent
 * copy of a clip already used, true puts the same clip there (a linked clip); a clip used nowhere is put there itself.
 * Who passes which: the Place buttons link by default (#624), while the grid button and the drag copy by default and link
 * with Alt (founder's decision 11). Pure: no DOM, no store.
 */
import type { Clip, Command, Placement, Project, Tick } from './generated/protocol'
import { placeFromSlot, duplicatePlacements } from './arrangement-commands'
import { duplicateClip, moveClip } from './clip-commands'
import { framesToTicks } from './audio/time'

/** The data type of a clip dragged from the pool. */
export const CLIP_DRAG_TYPE = 'application/x-faulk-clip'

/**
 * A second type naming the clip's track: a drag's data is hidden until the drop, but its types are not, so a lane or a
 * cell of another track can refuse the drag while it hovers (types are lower-cased by the browser).
 */
export const clipTrackType = (track: string): string => `application/x-faulk-clip-track-${track.toLowerCase()}`

export interface PoolClip {
  id: string
  clip: Clip
  /** Its placements in the arrangement, earliest first. */
  placements: { id: string; start: number; length: number }[]
  /** The scene of its launch-grid slot. */
  slot: string | null
  /** Placements plus the slot: more than one is a linked clip. */
  uses: number
  midi: boolean
}

/**
 * The clips of a track, by name then id. Empty for a track the project does not have. An audio placement's length is its
 * frames at the project's tempo (AU-6).
 */
export function poolOf(project: Pick<Project, 'tracks'> & Partial<Pick<Project, 'tempo'>>, track: string): PoolClip[] {
  const t = project.tracks[track]
  if (!t) return []
  const bpm = project.tempo?.[0]?.bpm ?? 120
  const placed = new Map<string, { id: string; start: number; length: number }[]>()
  for (const [id, p] of Object.entries(t.placements as Record<string, Placement | undefined>)) {
    if (!p) continue
    const list = placed.get(p.clip) ?? []
    const content = t.clips[p.clip]?.content
    const length =
      'ticks' in p.span ? p.span.ticks.length : content && 'audio' in content ? framesToTicks(p.span.frames.length, content.audio.media.sample_rate, bpm) : 0
    list.push({ id, start: p.start, length })
    placed.set(p.clip, list)
  }
  const out: PoolClip[] = []
  for (const [id, clip] of Object.entries(t.clips as Record<string, Clip | undefined>)) {
    if (!clip) continue
    const placements = (placed.get(id) ?? []).sort((a, b) => a.start - b.start)
    const slot = clip.slot ?? null
    out.push({ id, clip, placements, slot, uses: placements.length + (slot ? 1 : 0), midi: 'midi' in clip.content })
  }
  return out.sort((a, b) => a.clip.name.localeCompare(b.clip.name) || (a.id < b.id ? -1 : 1))
}

/**
 * The name each row shows, by clip id (#624): the clip's name (`fallback` for none), and a number after it when another
 * clip of the pool has the same name, so that two rows never read alike.
 */
export function poolNames(pool: readonly PoolClip[], fallback: string): Record<string, string> {
  const total: Record<string, number> = {}
  for (const e of pool) total[e.clip.name] = (total[e.clip.name] ?? 0) + 1
  const seen: Record<string, number> = {}
  const out: Record<string, string> = {}
  for (const e of pool) {
    const n = (seen[e.clip.name] = (seen[e.clip.name] ?? 0) + 1)
    const base = e.clip.name || fallback
    out[e.id] = (total[e.clip.name] ?? 0) > 1 ? `${base} (${n})` : base
  }
  return out
}

/** The command that puts a pool clip (MIDI or audio, AU-6) in the arrangement at `start`. */
export function placeCommand(entry: PoolClip, start: Tick, linked: boolean): Command | null {
  // A grid clip: the grid's own command, a copy by default.
  if (entry.slot) return placeFromSlot(entry.id, start, linked)
  const first = entry.placements[0]
  // Used in the arrangement already: a copy of its first placement by default, the same clip with `linked`.
  // A copy that would land on its own source (the playhead inside it) goes right after it instead of cutting it.
  if (first && !linked) {
    const inside = start >= first.start && start < first.start + first.length
    return duplicatePlacements([first.id], false, inside ? first.length : start - first.start)
  }
  return { type: 'arrangement.place', clip: entry.id, start }
}

/**
 * The first scene, in display order, whose slot in `track` is free; `null` when every slot is taken (or there is no
 * scene).
 */
export function freeScene(project: Pick<Project, 'tracks' | 'scenes'>, track: string): string | null {
  const t = project.tracks[track]
  if (!t) return null
  const taken = new Set(Object.values(t.clips as Record<string, Clip | undefined>).map((c) => c?.slot ?? null))
  const scenes = Object.entries(project.scenes)
    .filter((e): e is [string, NonNullable<(typeof e)[1]>] => e[1] !== undefined)
    .sort(([a, x], [b, y]) => (x.order < y.order ? -1 : x.order > y.order ? 1 : a < b ? -1 : 1))
  return scenes.find(([id]) => !taken.has(id))?.[0] ?? null
}

/**
 * The command that puts a pool clip (MIDI or audio, AU-6) in the slot of `scene` (free, in its track): the clip itself, or
 * an independent copy of a clip already used.
 */
export function gridCommand(entry: PoolClip, scene: string, linked: boolean): Command | null {
  // Used nowhere, or linked on purpose (a clip of the arrangement only): the clip itself goes into the slot.
  if (entry.uses === 0 || (linked && !entry.slot)) return moveClip(entry.id, scene, null)
  // Used already: an independent copy in the slot.
  return duplicateClip(entry.id, scene, null)
}
