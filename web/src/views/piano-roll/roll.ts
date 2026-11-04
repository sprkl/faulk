/**
 * The maths of the piano roll: where a note sits on the grid, which note a pointer hits, and what a
 * move, a resize, a velocity drag or a drawn note become as note values. Pure functions, no DOM:
 * `PianoRollView.svelte` forwards the pointer and sends the result as one command. Notes have no
 * identifier (CT-4): a note is addressed by its whole value, and `noteKey` is that value as a string.
 */

import { tickToX, type View } from '../../lib/components/timegrid/grid'
import type { Note } from '../../lib/generated/protocol'
import { PPQ } from '../transport/time'

/** The lowest and the highest MIDI key. */
export const MIN_PITCH = 0
export const MAX_PITCH = 127
/** The pitches shown: all of them. */
export const PITCH_COUNT = MAX_PITCH - MIN_PITCH + 1
export const MIN_VELOCITY = 1
export const MAX_VELOCITY = 127
/** The velocity of a note drawn before the user has set one. */
export const DEFAULT_VELOCITY = 100
/** The length of a note drawn with the snap off, before any has been drawn or resized: a sixteenth. */
export const SIXTEENTH = PPQ / 4
/** The grab zone at the right edge of a note, in pixels: a pointer in it resizes. */
export const EDGE_PX = 8
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
/**
 * The scroll that puts the middle of the clip's notes in sight: around the notes' pitches, or middle C when empty.
 * `sounds` are the keys the instrument answers to (a drum kit's): they are in sight too, and alone when the clip is empty.
 */
export function initialScrollY(notes: readonly Note[], height: number, layout: Pick<Layout, 'rowHeight' | 'top'>, sounds: readonly number[] = []): number {
  let low = MAX_PITCH
  let high = MIN_PITCH
  for (const key of sounds) {
    low = Math.min(low, key)
    high = Math.max(high, key)
  }
  for (const n of notes) {
    low = Math.min(low, n.pitch)
    high = Math.max(high, n.pitch)
  }
  const middle = notes.length === 0 && sounds.length === 0 ? 60 : (low + high) / 2
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
    // The right edge belongs to the next cell: a click right after a note draws a new one.
    if (x < r.x || x >= r.x + r.w) continue
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
export const rectBetween = (ax: number, ay: number, bx: number, by: number): Rect => ({
  x: Math.min(ax, bx),
  y: Math.min(ay, by),
  w: Math.abs(bx - ax),
  h: Math.abs(by - ay),
})

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

/** The draw velocity typed in the toolbar (#527): rounded and kept between 1 and 127; `fallback` when it is not a number. */
export function parseDrawVelocity(text: string, fallback: number): number {
  const value = text.trim() === '' ? Number.NaN : Number(text)
  return Number.isFinite(value) ? clamp(Math.round(value), MIN_VELOCITY, MAX_VELOCITY) : fallback
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

/**
 * The velocity step of Page Up and Page Down (#494): each note of the selection by `delta`, clamped to 1–127 on its own (a note
 * at 127 stays there while the others rise). When clamping alone would make two notes the same (the clip refuses that), the
 * notes move together as `shiftVelocity` does. Only the notes that change; empty when none can.
 */
export function stepVelocity(all: readonly Note[], selected: readonly Note[], delta: number): Change[] {
  const each = selected.map((from) => ({ from, to: { ...from, velocity: Math.round(clamp(from.velocity + finite(delta, 0), MIN_VELOCITY, MAX_VELOCITY)) } }))
  const ok = acceptable(all, each) ?? acceptable(all, shiftVelocity(selected, delta))
  return ok ?? []
}

/** The length of the next drawn note: the last one drawn or resized, else the snap step (a sixteenth when the snap is off). */
export const defaultLength = (last: number | null, step: number | null): number => last ?? (step !== null && step > 0 ? step : SIXTEENTH)

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
export function drawnNote(
  pressTick: number,
  pointerTick: number | null,
  pitch: number,
  velocity: number,
  step: number | null,
  lastLength: number,
  clipLength: number,
): Note | null {
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

/**
 * The note the lane acts on at `x`: among the notes that share the step of the bar under the pointer, the
 * selected one (the highest when several are), else the highest note of the step. Null when no bar is there.
 */
export function velocityTarget(notes: readonly Note[], x: number, view: View, picked: ReadonlySet<string>, reach = 4): number | null {
  const near = hitVelocity(notes, x, view, reach)
  if (near === null) return null
  const start = notes[near]!.start
  let best: number | null = null
  let bestChosen = false
  for (let i = 0; i < notes.length; i++) {
    const n = notes[i]!
    if (n.start !== start) continue
    const chosen = picked.has(noteKey(n))
    if (best === null || (chosen && !bestChosen) || (chosen === bestChosen && n.pitch > notes[best]!.pitch)) {
      best = i
      bestChosen = chosen
    }
  }
  return best
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

// --- PN-2: quantize, transpose, copy and paste --------------------------------------------------

/** The grids the quantize field offers, in ticks (960 per quarter note); triplets divide the beat in three. */
export const QUANTIZE_GRIDS = [
  { id: 'quarter', ticks: PPQ },
  { id: 'eighth', ticks: PPQ / 2 },
  { id: 'sixteenth', ticks: PPQ / 4 },
  { id: 'thirty-second', ticks: PPQ / 8 },
  { id: 'eighth-triplet', ticks: PPQ / 3 },
  { id: 'sixteenth-triplet', ticks: PPQ / 6 },
] as const
export type QuantizeGrid = (typeof QUANTIZE_GRIDS)[number]['id']
export const DEFAULT_QUANTIZE_GRID: QuantizeGrid = 'sixteenth'
export const DEFAULT_QUANTIZE_STRENGTH = 100
/** The most bars a clip can be given (the length field's limit). */
export const MAX_BARS = 256

/**
 * Where `clip.notes.quantize` puts a start: the same maths as the command (the nearest line of the grid, halfway to the
 * later one, by `strength` percent of the distance, halfway rounding away from zero), kept inside the clip for a note of
 * `length`. The page needs it to keep the selection on the notes the command moves.
 */
export function quantizedStart(start: number, grid: number, strength: number, length: number, clipLength: number): number {
  const g = Math.max(1, Math.round(grid))
  const nearest = Math.floor((start + Math.floor(g / 2)) / g) * g
  const scaled = (nearest - start) * clamp(Math.round(finite(strength, 0)), 0, 100)
  const share = Math.floor((Math.abs(scaled) + 50) / 100)
  const moved = start + (scaled < 0 ? -share : share)
  return clamp(moved, 0, Math.max(0, clipLength - length))
}

/** The changes `clip.notes.quantize` makes to these notes. */
export function quantizeChanges(selected: readonly Note[], grid: number, strength: number, clipLength: number): Change[] {
  return selected.map((from) => ({ from, to: { ...from, start: quantizedStart(from.start, grid, strength, from.length, clipLength) } }))
}

/** What the quantize says: how many notes moved, or that they were all on the grid already, or that there is nothing to quantize. */
export function quantizeSaid(
  moved: number,
  total: number,
): { key: 'pianoRoll.quantizeMoved' | 'pianoRoll.quantizeOnGrid' | 'pianoRoll.quantizeNone'; n: number } {
  if (total === 0) return { key: 'pianoRoll.quantizeNone', n: 0 }
  return moved === 0 ? { key: 'pianoRoll.quantizeOnGrid', n: 0 } : { key: 'pianoRoll.quantizeMoved', n: moved }
}

/** Transposes notes by `semitones`; `null` when one would leave the keyboard (the command refuses the whole batch). */
export function transposeChanges(selected: readonly Note[], semitones: number): Change[] | null {
  const shift = Math.round(finite(semitones, 0))
  if (selected.length === 0 || shift === 0) return []
  if (selected.some((n) => n.pitch + shift < MIN_PITCH || n.pitch + shift > MAX_PITCH)) return null
  return selected.map((from) => ({ from, to: { ...from, pitch: from.pitch + shift } }))
}

/** Notes put aside by copy or cut: their starts are relative to the earliest one, so a paste can land anywhere. */
export function toBoard(notes: readonly Note[]): Note[] {
  if (notes.length === 0) return []
  const first = Math.min(...notes.map((n) => n.start))
  return notes.map((n) => ({ ...n, start: n.start - first }))
}

/** The notes of a board put at `at`. */
export const fromBoard = (board: readonly Note[], at: number): Note[] => board.map((n) => ({ ...n, start: n.start + Math.max(0, Math.round(finite(at, 0))) }))

/** The end of the last note, or 0. */
export const endOf = (notes: readonly Note[]): number => notes.reduce((end, n) => Math.max(end, n.start + n.length), 0)

/**
 * The length a clip needs to hold notes that end at `end`: its own when they fit, else the next whole bar. `null` when
 * that is longer than a clip can be.
 */
export function lengthFor(end: number, clipLength: number, bar: number): number | null {
  if (end <= clipLength) return clipLength
  const length = Math.ceil(end / bar) * bar
  return length > MAX_BARS * bar ? null : length
}

/**
 * The span a duplicate shifts by: from the start of the bar of the first note to the last end of
 * the selection, rounded up to whole bars (at least one). Same rule as `clip.notes.duplicate` without `offset`.
 */
export function duplicateOffset(selected: readonly Note[], bar: number): number {
  if (selected.length === 0 || !(bar > 0)) return 0
  const from = Math.floor(Math.min(...selected.map((n) => n.start)) / bar) * bar
  return Math.max(1, Math.ceil((endOf(selected) - from) / bar)) * bar
}

/**
 * The step a note is drawn on. The ruler's Auto step follows the zoom and may get finer than a
 * sixteenth, which would draw slivers: for drawing it stops at a sixteenth. A step the musician
 * picked, or no snapping, is kept as is.
 */
export const drawStep = (auto: boolean, step: number | null): number | null => (auto && step !== null ? Math.max(step, SIXTEENTH) : step)
