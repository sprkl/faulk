/**
 * The cover of a Journey entry: an abstract sound object, from the same family as the landing
 * page's (an ink stage, blurred colour foci, one object in the middle, grain), generated from the
 * entry's data. Pure and deterministic: the same entry always gives the same cover, so no image is
 * stored and the prerendered SVG never changes between builds.
 *
 * What shapes it:
 * - the seed (`seedOf`): the date, the slug and the musical data, hashed;
 * - one blurred focus per track, in the track's colour;
 * - the object's form from the mode (minor modes an orb, modal ones a ring, major ones a tile),
 *   its angle from the tonic;
 * - faint rings around it, closer together the faster the tempo;
 * - along the bottom, the arrangement as columns of dots: one column per bar, as many dots as
 *   parts playing in its section.
 */

import { TONICS, type JourneyEntry, type Mode } from './types'

/** The cover's viewBox is ART_SIZE × ART_SIZE. */
export const ART_SIZE = 400

export interface Focus {
  x: number
  y: number
  r: number
  color: string
  opacity: number
}

export type ObjectKind = 'orb' | 'ring' | 'tile'

export interface SoundObject {
  kind: ObjectKind
  x: number
  y: number
  /** Radius (orb, ring) or half side (tile). */
  size: number
  /** Degrees. */
  rotation: number
  /** Light, main and deep colours of its gradient. */
  colors: [string, string, string]
}

export interface Ring {
  r: number
  opacity: number
}

export interface Dot {
  x: number
  y: number
  r: number
  opacity: number
}

export interface Art {
  seed: number
  foci: Focus[]
  object: SoundObject
  rings: Ring[]
  form: Dot[]
}

/** 32-bit FNV-1a of a string. */
export function hash(text: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return h >>> 0
}

/** Mulberry32: a small seeded generator, 0 ≤ x < 1. */
export function rng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** The entry's visual seed: its day and its music. */
export function seedOf(entry: JourneyEntry): number {
  const music = [
    entry.bpm,
    entry.key.tonic,
    entry.key.mode,
    entry.bars,
    ...entry.tracks.map((t) => t.color.toLowerCase()),
    ...entry.sections.map((s) => `${s.bars}x${s.parts}`),
  ]
  return hash(`${entry.date}|${entry.slug}|${music.join('|')}`)
}

const KIND: Record<Mode, ObjectKind> = { minor: 'orb', phrygian: 'orb', dorian: 'ring', mixolydian: 'ring', major: 'tile', lydian: 'tile' }

/** Mixes two `#rrggbb` colours, `t` of the way from `a` to `b`. */
export function mix(a: string, b: string, t: number): string {
  const p = (c: string, i: number) => parseInt(c.slice(1 + i * 2, 3 + i * 2), 16)
  const ch = (i: number) =>
    Math.round(p(a, i) + (p(b, i) - p(a, i)) * t)
      .toString(16)
      .padStart(2, '0')
  return `#${ch(0)}${ch(1)}${ch(2)}`
}

const round = (n: number) => Math.round(n * 10) / 10
const DEEP = '#2a0e5c'

export function artOf(entry: JourneyEntry): Art {
  const seed = seedOf(entry)
  const r = rng(seed)
  const c = ART_SIZE / 2
  const tracks = entry.tracks.slice(0, 8)

  // Foci: spread around the centre, each track at its own angle, a little off a regular spread.
  const turn = r() * Math.PI * 2
  const foci: Focus[] = tracks.map((t, i) => {
    const a = turn + (i / tracks.length) * Math.PI * 2 + (r() - 0.5) * 0.6
    const d = 70 + r() * 80
    return {
      x: round(c + Math.cos(a) * d),
      y: round(c - 20 + Math.sin(a) * d * 0.8),
      r: round(70 + r() * 60),
      color: t.color.toLowerCase(),
      opacity: round(0.55 + r() * 0.35),
    }
  })

  // The object: the two most distant colours of the palette as its gradient.
  const tonic = Math.max(0, TONICS.indexOf(entry.key.tonic))
  const main = tracks[Math.floor(r() * tracks.length)].color.toLowerCase()
  const second = tracks[(tracks.findIndex((t) => t.color.toLowerCase() === main) + Math.ceil(tracks.length / 2)) % tracks.length].color.toLowerCase()
  const object: SoundObject = {
    kind: KIND[entry.key.mode],
    x: c,
    y: round(c - 24),
    size: round(62 + r() * 18),
    rotation: tonic * 30,
    colors: [mix(main, '#ffffff', 0.55), mix(main, second, 0.35), mix(second, DEEP, 0.55)],
  }

  // Tempo rings: 2400 / bpm px apart (122 bpm ≈ 20 px, 76 bpm ≈ 32 px).
  const gap = 2400 / entry.bpm
  const rings: Ring[] = []
  for (let k = 1, rad = object.size + gap; rad < c * 0.95 && k <= 9; k++, rad += gap) rings.push({ r: round(rad), opacity: round(0.22 - k * 0.02) })

  // The form: one column per bar, its height the density of its section.
  const left = 36
  const width = ART_SIZE - left * 2
  const step = width / Math.max(1, entry.bars - 1)
  const maxParts = Math.max(...entry.sections.map((s) => s.parts))
  const form: Dot[] = []
  let bar = 0
  for (const s of entry.sections) {
    for (let b = 0; b < s.bars; b++, bar++) {
      const x = round(left + bar * step)
      for (let k = 0; k < s.parts; k++)
        form.push({ x, y: round(ART_SIZE - 30 - k * 6), r: entry.bars > 96 ? 0.9 : 1.3, opacity: round(0.35 + (0.6 * s.parts) / maxParts - k * 0.03) })
    }
  }

  return { seed, foci, object, rings, form }
}
