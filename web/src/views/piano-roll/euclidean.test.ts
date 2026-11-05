import { describe, expect, it } from 'vitest'
import type { Note } from '../../lib/generated/protocol'
import { PPQ } from '../transport/time'
import { DEFAULT_GATE, euclideanFault, euclideanHits, euclideanNotes, euclideanPattern, euclideanPlan, parsePitch, type EuclideanSettings } from './euclidean'

const SIXTEENTH = PPQ / 4
const BAR = 4 * PPQ

const settings = (over: Partial<EuclideanSettings> = {}): EuclideanSettings => ({
  steps: 8,
  hits: 3,
  rotation: 0,
  grid: SIXTEENTH,
  pitch: 36,
  velocity: 100,
  gate: DEFAULT_GATE,
  fill: false,
  replace: true,
  ...over,
})

const text = (pattern: boolean[]) => pattern.map((on) => (on ? 'x' : '.')).join('')

describe('euclideanPattern and euclideanHits (#694)', () => {
  it('E(3, 8) puts hits on 0, 3 and 6', () => {
    expect(euclideanHits(8, 3, 0)).toEqual([0, 3, 6])
    expect(text(euclideanPattern(8, 3))).toBe('x..x..x.')
  })

  it("E(5, 8) is the standard Bjorklund pattern x.xx.xx. (the issue's 0, 2, 4, 5, 7 is one of its rotations)", () => {
    expect(text(euclideanPattern(8, 5))).toBe('x.xx.xx.')
    expect(euclideanHits(8, 5, 0)).toEqual([0, 2, 3, 5, 6])
    // 0, 2, 4, 5, 7 is the same necklace: the standard pattern turned by 2 steps.
    expect(euclideanHits(8, 5, 2)).toEqual([0, 2, 4, 5, 7])
  })

  it('matches the known patterns', () => {
    expect(text(euclideanPattern(12, 7))).toBe('x.xx.x.xx.x.')
    expect(text(euclideanPattern(16, 5))).toBe('x..x..x..x..x...')
    expect(text(euclideanPattern(5, 2))).toBe('x.x..')
    expect(text(euclideanPattern(4, 3))).toBe('xxx.')
  })

  it('turns the pattern later by the rotation, round the steps', () => {
    const base = euclideanHits(16, 5, 0)
    expect(euclideanHits(16, 5, 2)).toEqual(base.map((h) => (h + 2) % 16).sort((a, b) => a - b))
    expect(euclideanHits(16, 5, 2)).toEqual([2, 5, 8, 11, 14])
    // A full turn is no turn; a negative one counts from the end.
    expect(euclideanHits(8, 3, 8)).toEqual([0, 3, 6])
    expect(euclideanHits(8, 3, -1)).toEqual([2, 5, 7])
  })

  it('N = M hits every step and N = 1 hits the first step only', () => {
    expect(euclideanHits(8, 8, 0)).toEqual([0, 1, 2, 3, 4, 5, 6, 7])
    expect(euclideanHits(8, 1, 0)).toEqual([0])
    expect(euclideanHits(8, 1, 3)).toEqual([3])
  })

  it('every pattern has exactly N hits and its hits are as even as they can be', () => {
    for (let steps = 2; steps <= 64; steps++) {
      for (let hits = 1; hits <= steps; hits++) {
        const slots = euclideanHits(steps, hits, 0)
        expect(slots).toHaveLength(hits)
        expect(slots[0]).toBe(0)
        // The gaps (round the circle) differ by one step at most.
        const gaps = slots.map((s, i) => (i + 1 < slots.length ? slots[i + 1]! : steps) - s)
        expect(Math.max(...gaps) - Math.min(...gaps)).toBeLessThanOrEqual(1)
      }
    }
  })

  it('gives silence for no hit and never loops on it', () => {
    expect(euclideanHits(8, 0, 0)).toEqual([])
    expect(euclideanHits(0, 3, 0)).toEqual([])
  })
})

describe('euclideanFault', () => {
  it('accepts the documented ranges and names the first field out of range', () => {
    expect(euclideanFault(settings())).toBeNull()
    expect(euclideanFault(settings({ steps: 1 }))).toBe('steps')
    expect(euclideanFault(settings({ steps: 65 }))).toBe('steps')
    expect(euclideanFault(settings({ steps: null }))).toBe('steps')
    expect(euclideanFault(settings({ steps: 2.5 }))).toBe('steps')
    expect(euclideanFault(settings({ hits: 0 }))).toBe('hits')
    expect(euclideanFault(settings({ hits: 9 }))).toBe('hits')
    expect(euclideanFault(settings({ rotation: 8 }))).toBe('rotation')
    expect(euclideanFault(settings({ rotation: -1 }))).toBe('rotation')
    expect(euclideanFault(settings({ pitch: 128 }))).toBe('pitch')
    expect(euclideanFault(settings({ pitch: null }))).toBe('pitch')
    expect(euclideanFault(settings({ velocity: 0 }))).toBe('velocity')
    expect(euclideanFault(settings({ gate: 0 }))).toBe('gate')
    expect(euclideanFault(settings({ gate: 101 }))).toBe('gate')
    expect(euclideanFault(settings({ steps: 64, hits: 64, rotation: 63 }))).toBeNull()
  })
})

describe('euclideanNotes', () => {
  it('writes one run at the start, a note per hit, as long as the gate says', () => {
    const notes = euclideanNotes(settings({ gate: 50 }), 0, BAR)
    expect(notes.map((n) => n.start)).toEqual([0, 3 * SIXTEENTH, 6 * SIXTEENTH])
    expect(notes.every((n) => n.pitch === 36 && n.velocity === 100 && n.length === SIXTEENTH / 2)).toBe(true)
  })

  it('shifts every hit by the rotation, in steps of the grid', () => {
    const plain = euclideanNotes(settings({ steps: 16, hits: 5 }), 0, BAR)
    const turned = euclideanNotes(settings({ steps: 16, hits: 5, rotation: 2 }), 0, BAR)
    expect(turned.map((n) => n.start)).toEqual(plain.map((n) => n.start + 2 * SIXTEENTH))
  })

  it('fills a clip of two bars in sixteenths: 32 steps, E(3, 8) four times', () => {
    const notes = euclideanNotes(settings({ fill: true }), 0, 2 * BAR)
    expect(notes).toHaveLength(12)
    expect(notes.map((n) => n.start / SIXTEENTH)).toEqual([0, 3, 6, 8, 11, 14, 16, 19, 22, 24, 27, 30])
  })

  it('fills from a start past the first step and cuts at the clip end', () => {
    const notes = euclideanNotes(settings({ fill: true }), 20 * SIXTEENTH, 2 * BAR)
    expect(notes.map((n) => n.start / SIXTEENTH)).toEqual([20, 23, 26, 28, 31])
    const last = euclideanNotes(settings({ gate: 100, grid: SIXTEENTH }), BAR - SIXTEENTH / 2, BAR)
    expect(last).toEqual([{ start: BAR - SIXTEENTH / 2, pitch: 36, length: SIXTEENTH / 2, velocity: 100 }])
  })

  it('leaves out the steps at or past the clip end and starts past it', () => {
    expect(euclideanNotes(settings(), 0, 4 * SIXTEENTH).map((n) => n.start / SIXTEENTH)).toEqual([0, 3])
    expect(euclideanNotes(settings(), BAR, BAR)).toEqual([])
  })

  it('gives at least a tick, and nothing for a refused setting', () => {
    expect(euclideanNotes(settings({ grid: 3, gate: 1 }), 0, BAR)[0]!.length).toBe(1)
    expect(euclideanNotes(settings({ hits: 20 }), 0, BAR)).toEqual([])
    expect(euclideanNotes(settings({ grid: 0 }), 0, BAR)).toEqual([])
  })
})

describe('euclideanPlan', () => {
  const note = (start: number, pitch = 36, length = SIXTEENTH / 2, velocity = 100): Note => ({ start, pitch, length, velocity })

  it('replaces the same-pitch notes that overlap the pattern and leaves the other pitches', () => {
    const made = euclideanNotes(settings(), 0, 2 * BAR)
    const existing = [note(SIXTEENTH), note(5 * SIXTEENTH, 36, 2 * SIXTEENTH), note(SIXTEENTH, 38), note(BAR, 36)]
    const plan = euclideanPlan(existing, made, settings(), 0, 2 * BAR)
    // The span is 8 steps (half a bar): the note on step 1 and the long one that reaches step 5 go, the other pitch and the next bar stay.
    expect(plan.remove).toEqual([note(SIXTEENTH), note(5 * SIXTEENTH, 36, 2 * SIXTEENTH)])
    expect(plan.add).toEqual(made)
  })

  it('replaces up to the clip end when it fills', () => {
    const s = settings({ fill: true })
    const made = euclideanNotes(s, 0, 2 * BAR)
    const plan = euclideanPlan([note(BAR + SIXTEENTH)], made, s, 0, 2 * BAR)
    expect(plan.remove).toEqual([note(BAR + SIXTEENTH)])
  })

  it('keeps a note that is exactly what the pattern writes, and writes it once', () => {
    const made = euclideanNotes(settings(), 0, BAR)
    const plan = euclideanPlan([made[1]!], made, settings(), 0, BAR)
    expect(plan.remove).toEqual([])
    expect(plan.add).toEqual([made[0], made[2]])
  })

  it('removes nothing without replace and still skips a note already there', () => {
    const s = settings({ replace: false })
    const made = euclideanNotes(s, 0, BAR)
    const plan = euclideanPlan([made[0]!, note(SIXTEENTH)], made, s, 0, BAR)
    expect(plan.remove).toEqual([])
    expect(plan.add).toEqual([made[1], made[2]])
  })

  it('does nothing for no notes', () => {
    expect(euclideanPlan([note(0)], [], settings(), 0, BAR)).toEqual({ remove: [], add: [] })
  })
})

describe('parsePitch', () => {
  it('reads a MIDI number or a note name as the roll names its keys', () => {
    expect(parsePitch('60')).toBe(60)
    expect(parsePitch(' 36 ')).toBe(36)
    expect(parsePitch('C4')).toBe(60)
    expect(parsePitch('c2')).toBe(36)
    expect(parsePitch('F#3')).toBe(54)
    expect(parsePitch('Bb1')).toBe(34)
    expect(parsePitch('C-1')).toBe(0)
    expect(parsePitch('G9')).toBe(127)
  })

  it('refuses anything else', () => {
    for (const text of ['', 'H2', 'C', '128', 'G#9', 'C-2', '3.5', '-4', 'C##4']) expect(parsePitch(text), text).toBeNull()
  })
})
