/**
 * The euclidean rhythm generator of the piano roll (#694): `hits` notes spread as evenly as possible over `steps` grid steps,
 * turned by a rotation, written at one pitch. A tool of the page: what it produces is `clip.notes.add` (and a
 * `clip.notes.remove` of the same-pitch notes it replaces, in one history group); an agent computes the same pattern itself.
 * Pure functions, no DOM.
 */

import type { Note } from '../../lib/generated/protocol'
import { MAX_PITCH, MAX_VELOCITY, MIN_PITCH, MIN_VELOCITY, noteKey } from './roll'

export const MIN_STEPS = 2
export const MAX_STEPS = 64
export const MIN_GATE = 1
export const MAX_GATE = 100
export const DEFAULT_GATE = 50

/**
 * The pattern of `hits` onsets over `steps` steps, as Bjorklund's algorithm builds it (the way it is usually written):
 * `hits` groups "x" and `steps - hits` groups ".", the groups of the shorter run are glued one by one to the longer run's,
 * and the leftover of the longer run is glued again, until at most one group is left over. The result starts with a hit.
 * E(3,8) = x..x..x. and E(5,8) = x.xx.xx. A `hits` of 0 or less gives silence, of `steps` or more every step.
 */
export function euclideanPattern(steps: number, hits: number): boolean[] {
  const total = Math.max(0, Math.floor(steps))
  const ones = Math.min(total, Math.max(0, Math.floor(hits)))
  let head: boolean[][] = Array.from({ length: ones }, () => [true])
  let tail: boolean[][] = Array.from({ length: total - ones }, () => [false])
  while (head.length > 0 && tail.length > 1) {
    const paired = Math.min(head.length, tail.length)
    const glued = Array.from({ length: paired }, (_, i) => [...head[i]!, ...tail[i]!])
    const rest = head.length > paired ? head.slice(paired) : tail.slice(paired)
    head = glued
    tail = rest
  }
  return [...head, ...tail].flat()
}

/** The steps of a hit, ascending, after turning the pattern `rotation` steps later (round the `steps`). */
export function euclideanHits(steps: number, hits: number, rotation: number): number[] {
  const pattern = euclideanPattern(steps, hits)
  const n = pattern.length
  if (n === 0) return []
  const turn = ((Math.floor(rotation) % n) + n) % n
  const out: number[] = []
  pattern.forEach((on, i) => {
    if (on) out.push((i + turn) % n)
  })
  return out.sort((a, b) => a - b)
}

/** The values of the generator's fields as they are typed: a number field is null when empty. */
export interface EuclideanDraft {
  steps: number | null
  hits: number | null
  rotation: number | null
  /** The id of a `QUANTIZE_GRIDS` entry. */
  grid: string
  /** A MIDI number or a note name; on a drum kit the key of the chosen sound, as a number. */
  pitch: string
  velocity: number | null
  gate: number | null
  fill: boolean
  /** "Add without replacing". */
  keep: boolean
}

/** What the generator's fields hold, read: numbers a person typed are `null` when they are not whole numbers. */
export interface EuclideanSettings {
  steps: number | null
  hits: number | null
  rotation: number | null
  /** The grid step in ticks. */
  grid: number
  pitch: number | null
  velocity: number | null
  /** The note length as a percentage of the step. */
  gate: number | null
  /** Repeat the pattern up to the clip's end rather than once. */
  fill: boolean
  /** Replace the notes of the same pitch under the pattern (else they stay). */
  replace: boolean
}

/** The field that is wrong, or null when every field is a value the generator accepts. */
export type EuclideanFault = 'steps' | 'hits' | 'rotation' | 'pitch' | 'velocity' | 'gate'

const whole = (value: number | null, min: number, max: number): value is number => value !== null && Number.isInteger(value) && value >= min && value <= max

/** The first field that is out of range (`steps` 2 to 64, `hits` 1 to steps, `rotation` 0 to steps - 1, a MIDI pitch and velocity, a gate of 1 to 100 %). */
export function euclideanFault(s: EuclideanSettings): EuclideanFault | null {
  if (!whole(s.steps, MIN_STEPS, MAX_STEPS)) return 'steps'
  if (!whole(s.hits, 1, s.steps)) return 'hits'
  if (!whole(s.rotation, 0, s.steps - 1)) return 'rotation'
  if (!whole(s.pitch, MIN_PITCH, MAX_PITCH)) return 'pitch'
  if (!whole(s.velocity, MIN_VELOCITY, MAX_VELOCITY)) return 'velocity'
  if (!whole(s.gate, MIN_GATE, MAX_GATE)) return 'gate'
  return null
}

/**
 * The notes of the pattern from `start`: one run of `steps` steps, or runs end to end up to the clip's end with `fill`. A
 * note starts on its step and lasts `gate` percent of it (at least a tick), cut at the clip's end; a step at or past the
 * end is left out. Empty for settings the generator refuses.
 */
export function euclideanNotes(s: EuclideanSettings, start: number, clipLength: number): Note[] {
  if (euclideanFault(s) !== null || !(s.grid >= 1)) return []
  const steps = s.steps!
  const slots = euclideanHits(steps, s.hits!, s.rotation!)
  const length = Math.max(1, Math.round((s.grid * s.gate!) / 100))
  const notes: Note[] = []
  for (let run = 0; ; run++) {
    const base = start + run * steps * s.grid
    if (base >= clipLength) break
    for (const slot of slots) {
      const at = Math.round(base + slot * s.grid)
      if (at >= clipLength) break
      notes.push({ start: at, pitch: s.pitch!, length: Math.min(length, clipLength - at), velocity: s.velocity! })
    }
    if (!s.fill) break
  }
  return notes
}

/** What the generator writes: the notes to remove (the same-pitch notes it replaces) and the ones to add. */
export interface EuclideanPlan {
  remove: Note[]
  add: Note[]
}

/**
 * The edit for `made` (from `euclideanNotes`) in a clip that holds `existing`. Replacing, the notes of the pattern's pitch
 * that overlap the pattern's span (from `start` to the end of its last run, to the clip's end with `fill`) are removed; a
 * note already there exactly as the pattern would write it stays, and is not written twice. Not replacing, nothing is
 * removed.
 */
export function euclideanPlan(existing: readonly Note[], made: readonly Note[], s: EuclideanSettings, start: number, clipLength: number): EuclideanPlan {
  if (made.length === 0) return { remove: [], add: [] }
  const there = new Set(existing.map(noteKey))
  const wanted = new Set(made.map(noteKey))
  const add = made.filter((n) => !there.has(noteKey(n)))
  if (!s.replace) return { remove: [], add }
  const end = s.fill ? clipLength : Math.min(clipLength, start + s.steps! * s.grid)
  const remove = existing.filter((n) => n.pitch === s.pitch && n.start < end && n.start + n.length > start && !wanted.has(noteKey(n)))
  return { remove, add }
}

const SEMITONES: Readonly<Record<string, number>> = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 }

/**
 * A pitch as a person types it: a MIDI number (60), or a note name (C4 is 60, as the roll's keyboard names it) with an optional
 * sharp (# or ♯) or flat (b or ♭). Null for anything else or a pitch out of the MIDI range.
 */
export function parsePitch(text: string): number | null {
  const trimmed = text.trim()
  if (/^\d{1,3}$/.test(trimmed)) {
    const n = Number(trimmed)
    return n <= MAX_PITCH ? n : null
  }
  const match = /^([A-Ga-g])([#♯b♭]?)(-?\d{1,2})$/.exec(trimmed)
  if (!match) return null
  const base = SEMITONES[match[1]!.toUpperCase()]!
  const shift = match[2] === '#' || match[2] === '♯' ? 1 : match[2] === 'b' || match[2] === '♭' ? -1 : 0
  const pitch = (Number(match[3]) + 1) * 12 + base + shift
  return pitch >= MIN_PITCH && pitch <= MAX_PITCH ? pitch : null
}
