/**
 * The maths of the piano roll: where a note sits on the grid, which note a pointer hits, and what a
 * move, a resize, a velocity drag or a drawn note become as note values. Pure functions, no DOM:
 * `PianoRollView.svelte` forwards the pointer and sends the result as one command. Notes have no
 * identifier (CT-4): a note is addressed by its whole value, and `noteKey` is that value as a string.
 */

import { tickToX, type View } from '../../lib/components/timegrid/grid'
import type { Note } from '../../lib/generated/protocol'

/** The lowest and the highest MIDI key. */
export const MIN_PITCH = 0
export const MAX_PITCH = 127
/** The pitches shown: all of them. */
export const PITCH_COUNT = MAX_PITCH - MIN_PITCH + 1
export const MIN_VELOCITY = 1
export const MAX_VELOCITY = 127
/** The velocity of a note drawn before the user has set one. */
export const DEFAULT_VELOCITY = 100
/** The grab zone at the right edge of a note, in pixels: a pointer in it resizes. */
export const EDGE_PX = 6
/** The least width of a note on screen, so that a very short note stays visible and hittable. */
export const MIN_NOTE_PX = 3

/** The vertical axis: row height, the scroll in pixels (0 shows the highest pitch at the top), and where the body starts. */
export interface Layout {
  rowHeight: number
  scrollY: number
  /** The top of the pitch rows in pixels from the top of the surface (under the ruler). */
  top: number
}

export const DEFAULT_ROW_HEIGHT = 16
export const MIN_ROW_HEIGHT = 8
export const MAX_ROW_HEIGHT = 32

/** A note as its value, the way `clip.notes.*` addresses it. */
export const noteKey = (n: Note): string => `${n.start}:${n.pitch}:${n.length}:${n.velocity}:${n.cents ?? 0}:${n.chance ?? 100}:${n.repeat ?? 1}`

const BLACK = new Set([1, 3, 6, 8, 10])
export const isBlackKey = (pitch: number): boolean => BLACK.has(((pitch % 12) + 12) % 12)
const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
/** The name of a key, middle C (60) being C4. */
export const pitchName = (pitch: number): string => `${NAMES[((pitch % 12) + 12) % 12]}${Math.floor(pitch / 12) - 1}`

const clamp = (value: number, low: number, high: number): number => Math.min(high, Math.max(low, value))
const finite = (value: number, fallback: number): number => (Number.isFinite(value) ? value : fallback)

/** The y of the top edge of a pitch's row. */
export const pitchTop = (pitch: number, layout: Layout): number => layout.top + (MAX_PITCH - pitch) * layout.rowHeight - layout.scrollY
/** The pitch whose row holds `y`, clamped to the keyboard. */
export const pitchAt = (y: number, layout: Layout): number =>
  clamp(MAX_PITCH - Math.floor((y - layout.top + layout.scrollY) / layout.rowHeight), MIN_PITCH, MAX_PITCH)

/** The greatest scroll: the lowest row touches the bottom of a surface `height` pixels tall. */
export const maxScrollY = (height: number, layout: Pick<Layout, 'rowHeight' | 'top'>): number =>
  Math.max(0, PITCH_COUNT * layout.rowHeight - Math.max(0, height - layout.top))
export const clampScrollY = (scrollY: number, height: number, layout: Pick<Layout, 'rowHeight' | 'top'>): number =>
  clamp(finite(scrollY, 0), 0, maxScrollY(height, layout))
/** The scroll that puts the middle of the clip's notes in sight: around the notes' pitches, or middle C when empty. */
export function initialScrollY(notes: readonly Note[], height: number, layout: Pick<Layout, 'rowHeight' | 'top'>): number {
  let low = MAX_PITCH
  let high = MIN_PITCH
  for (const n of notes) {
    low = Math.min(low, n.pitch)
    high = Math.max(high, n.pitch)
  }
  const middle = notes.length === 0 ? 60 : (low + high) / 2
  const y = (MAX_PITCH - middle) * layout.rowHeight - (height - layout.top) / 2
  return clampScrollY(y, height, layout)
}

export interface Rect {
  x: number
  y: number
  w: number
  h: number
}

/** The rectangle of a note on the surface, in CSS pixels. */
export function noteRect(note: Note, view: View, layout: Layout): Rect {
  const x = tickToX(note.start, view)
  return { x, y: pitchTop(note.pitch, layout), w: Math.max(MIN_NOTE_PX, note.length * view.pxPerTick), h: layout.rowHeight }
}

export type Zone = 'body' | 'end'
export interface Hit {
  /** Index in the array that was searched. */
  index: number
  zone: Zone
}

/**
 * The note under a pointer, the last one first (it is drawn last, so on top). A pointer within
 * `EDGE_PX` of the right edge of a note wide enough to keep a body is on its end: it resizes.
 */
export function hitNote(notes: readonly Note[], x: number, y: number, view: View, layout: Layout): Hit | null {
  const pitch = pitchAt(y, layout)
  if (y < layout.top) return null
  for (let i = notes.length - 1; i >= 0; i--) {
    const n = notes[i]!
    if (n.pitch !== pitch) continue
    const r = noteRect(n, view, layout)
    if (x < r.x || x > r.x + r.w) continue
    return { index: i, zone: r.w >= EDGE_PX * 2 && x >= r.x + r.w - EDGE_PX ? 'end' : 'body' }
  }
  return null
}

/** The keys of the notes whose rectangle meets the one dragged. */
export function notesInRect(notes: readonly Note[], area: Rect, view: View, layout: Layout): string[] {
  const keys: string[] = []
  const right = area.x + area.w
  const bottom = area.y + area.h
  for (const n of notes) {
    const r = noteRect(n, view, layout)
    if (r.x <= right && r.x + r.w >= area.x && r.y <= bottom && r.y + r.h >= area.y) keys.push(noteKey(n))
  }
  return keys
}

/** The rectangle between two corners, whichever way it was dragged. */
export const rectBetween = (ax: number, ay: number, bx: number, by: number): Rect => ({ x: Math.min(ax, bx), y: Math.min(ay, by), w: Math.abs(bx - ax), h: Math.abs(by - ay) })

/** A note that becomes another, the shape of `clip.notes.update`. */
export interface Change {
  from: Note
  to: Note
}

/**
 * The changes that `clip.notes.update` takes: those that change something, or `null` when the
 * command would refuse them (two arrive at the same value, or one arrives on a note that stays).
 * Checked here so that a drag the clip cannot take is dropped instead of sent.
 */
export function acceptable(all: readonly Note[], changes: readonly Change[]): Change[] | null {
  const real = changes.filter((c) => noteKey(c.from) !== noteKey(c.to))
  const leaving = new Set(real.map((c) => noteKey(c.from)))
  const staying = new Set<string>()
  for (const n of all) if (!leaving.has(noteKey(n))) staying.add(noteKey(n))
  const arriving = new Set<string>()
  for (const c of real) {
    const key = noteKey(c.to)
    if (staying.has(key) || arriving.has(key)) return null
    arriving.add(key)
  }
  return real
}

/** Moves notes by `dTick` and `dPitch`, both cut back so that the whole group stays in the clip and on the keyboard. */
export function moveNotes(selected: readonly Note[], dTick: number, dPitch: number, clipLength: number): Change[] {
  if (selected.length === 0) return []
  let minStart = Infinity
  let maxEnd = -Infinity
  let minPitch = Infinity
  let maxPitch = -Infinity
  for (const n of selected) {
    minStart = Math.min(minStart, n.start)
    maxEnd = Math.max(maxEnd, n.start + n.length)
    minPitch = Math.min(minPitch, n.pitch)
    maxPitch = Math.max(maxPitch, n.pitch)
  }
  const dt = Math.round(clamp(finite(dTick, 0), -minStart, Math.max(-minStart, clipLength - maxEnd)))
  const dp = Math.round(clamp(finite(dPitch, 0), MIN_PITCH - minPitch, MAX_PITCH - maxPitch))
  return selected.map((from) => ({ from, to: { ...from, start: from.start + dt, pitch: from.pitch + dp } }))
}

/** Changes the length of notes by `dLength`, cut back so that none gets shorter than one tick or ends past the clip. */
export function resizeNotes(selected: readonly Note[], dLength: number, clipLength: number): Change[] {
  if (selected.length === 0) return []
  let minLength = Infinity
  let maxEnd = -Infinity
  for (const n of selected) {
    minLength = Math.min(minLength, n.length)
    maxEnd = Math.max(maxEnd, n.start + n.length)
  }
  const low = 1 - minLength
  const dl = Math.round(clamp(finite(dLength, 0), low, Math.max(low, clipLength - maxEnd)))
  return selected.map((from) => ({ from, to: { ...from, length: from.length + dl } }))
}

/** Velocity from a position in the lane: the top is the loudest. */
export function velocityAt(y: number, laneTop: number, laneHeight: number): number {
  return velocityOf(velocityPosition(y, laneTop, laneHeight))
}

/**
 * What the default-velocity reset does to the chosen notes: each one not already at it takes it. Null
 * when the clip would refuse the whole change (two stacked notes that differ only in velocity would
 * become the same note), and an empty list when there is nothing to do.
 */
export function resetVelocities(all: readonly Note[], chosen: readonly Note[]): Change[] | null {
  const wanted = chosen.filter((n) => n.velocity !== DEFAULT_VELOCITY).map((from) => ({ from, to: { ...from, velocity: DEFAULT_VELOCITY } }))
  return acceptable(all, wanted)
}

/** The lane's position (0 at the bottom, 1 at the top) under a height, not rounded: the drag of `continuous.ts` works on it. */
export function velocityPosition(y: number, laneTop: number, laneHeight: number): number {
  return clamp(finite(1 - (y - laneTop) / Math.max(1, laneHeight), 0), 0, 1)
}

/** The velocity of a lane position: the inverse of `velocityPosition`, rounded and kept between 1 and 127. */
export function velocityOf(position: number): number {
  return clamp(Math.round(finite(position, 0) * MAX_VELOCITY), MIN_VELOCITY, MAX_VELOCITY)
}

/** Where the default velocity sits in the lane, held by the magnet. */
export const VELOCITY_HOME = DEFAULT_VELOCITY / MAX_VELOCITY

/**
 * Moves velocities together by one amount, cut back so that all stay between 1 and 127: the same
 * shift for every note keeps notes that differ only in velocity apart (clamping each alone could
 * make two of them the same note, which the clip refuses).
 */
export function shiftVelocity(selected: readonly Note[], delta: number): Change[] {
  if (selected.length === 0) return []
  let low = Infinity
  let high = -Infinity
  for (const n of selected) {
    low = Math.min(low, n.velocity)
    high = Math.max(high, n.velocity)
  }
  const shift = Math.round(clamp(finite(delta, 0), MIN_VELOCITY - low, Math.max(MIN_VELOCITY - low, MAX_VELOCITY - high)))
  return selected.map((from) => ({ from, to: { ...from, velocity: from.velocity + shift } }))
}

/** The tick of a grid cell: the pointer's tick rounded down to the step (a click inside a cell draws at its start). */
export function floorTick(tick: number, step: number | null): number {
  const safe = Math.max(0, finite(tick, 0))
  return step !== null && step > 0 ? Math.floor(safe / step) * step : Math.round(safe)
}

/**
 * The note a click or a drag draws: it starts at the cell of the pressed tick and ends at the
 * snapped tick of the pointer, at least one step long (the last length used when the pointer did
 * not move), and always inside the clip. `null` when the press is past the clip's end.
 */
export function drawnNote(pressTick: number, pointerTick: number | null, pitch: number, velocity: number, step: number | null, lastLength: number, clipLength: number): Note | null {
  const start = floorTick(pressTick, step)
  if (start >= clipLength) return null
  const least = step !== null && step > 0 ? step : 1
  const dragged = pointerTick !== null && pointerTick - start >= least
  const length = clamp(dragged ? Math.round(pointerTick - start) : Math.max(1, Math.round(lastLength)), 1, clipLength - start)
  return { start, pitch: clamp(Math.round(pitch), MIN_PITCH, MAX_PITCH), length, velocity: clamp(Math.round(velocity), MIN_VELOCITY, MAX_VELOCITY) }
}

/** The note whose velocity bar is under `x` in the lane (within `reach` pixels), the last one first. */
export function hitVelocity(notes: readonly Note[], x: number, view: View, reach = 4): number | null {
  for (let i = notes.length - 1; i >= 0; i--) {
    if (Math.abs(tickToX(notes[i]!.start, view) + 1 - x) <= reach) return i
  }
  return null
}

/** The selection with `key` toggled, and whether it is in it now. */
export function toggled(picked: ReadonlySet<string>, key: string): { picked: ReadonlySet<string>; on: boolean } {
  const on = !picked.has(key)
  return { picked: new Set(on ? [...picked, key] : [...picked].filter((k) => k !== key)), on }
}

/** The selection after changes: the notes that left are out, the notes that arrived are in. */
export function afterChanges(picked: ReadonlySet<string>, changes: readonly Change[]): ReadonlySet<string> {
  const left = new Set(changes.map((c) => noteKey(c.from)))
  return new Set([...[...picked].filter((k) => !left.has(k)), ...changes.map((c) => noteKey(c.to))])
}

/** Whether a note lies inside a clip of this length, as `clip.notes.*` requires. */
export const fits = (note: Note, clipLength: number): boolean =>
  note.start >= 0 &&
  note.length >= 1 &&
  note.start + note.length <= clipLength &&
  note.pitch >= MIN_PITCH &&
  note.pitch <= MAX_PITCH &&
  note.velocity >= MIN_VELOCITY &&
  note.velocity <= MAX_VELOCITY

/**
 * Whether changes computed during a gesture still apply to the clip as it is when the pointer is
 * released: every note they replace is still in it, and every note they bring fits its length. The
 * clip may have changed under the drag (an undo, an agent), and a refused command would lose the drag.
 */
export function stillApplies(all: readonly Note[], changes: readonly Change[], clipLength: number): boolean {
  const present = new Set(all.map(noteKey))
  return changes.every((c) => present.has(noteKey(c.from)) && fits(c.to, clipLength))
}
