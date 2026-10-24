import { describe, expect, it } from 'vitest'
import type { Note } from '../../lib/generated/protocol'
import {
  acceptable,
  clampScrollY,
  drawnNote,
  fits,
  floorTick,
  hitNote,
  hitVelocity,
  initialScrollY,
  isBlackKey,
  maxScrollY,
  moveNotes,
  noteKey,
  notesInRect,
  pitchAt,
  pitchName,
  pitchTop,
  rectBetween,
  resizeNotes,
  shiftVelocity,
  stillApplies,
  resetVelocities,
  velocityAt,
  velocityOf,
  velocityPosition,
  VELOCITY_HOME,
  DEFAULT_VELOCITY,
  type Layout,
} from './roll'

const view = { pxPerTick: 0.1, scrollTick: 0 }
const layout: Layout = { rowHeight: 16, scrollY: 0, top: 28 }
const note = (start: number, pitch: number, length = 480, velocity = 100): Note => ({ start, pitch, length, velocity })

describe('pitch axis', () => {
  it('puts the highest pitch at the top and reads a y back to its pitch', () => {
    expect(pitchTop(127, layout)).toBe(28)
    expect(pitchTop(126, layout)).toBe(44)
    for (const pitch of [0, 1, 60, 126, 127]) expect(pitchAt(pitchTop(pitch, layout) + 5, layout)).toBe(pitch)
    // Outside the keyboard it saturates.
    expect(pitchAt(-500, layout)).toBe(127)
    expect(pitchAt(1e6, layout)).toBe(0)
  })

  it('follows the vertical scroll', () => {
    const scrolled = { ...layout, scrollY: 160 }
    expect(pitchTop(117, scrolled)).toBe(28)
    expect(pitchAt(30, scrolled)).toBe(117)
  })

  it('names the keys with middle C as C4 and knows the black ones', () => {
    expect(pitchName(60)).toBe('C4')
    expect(pitchName(61)).toBe('C#4')
    expect(pitchName(0)).toBe('C-1')
    expect(pitchName(127)).toBe('G9')
    expect([1, 3, 6, 8, 10, 61, 70].every(isBlackKey)).toBe(true)
    expect([0, 2, 4, 5, 7, 9, 11, 60].some(isBlackKey)).toBe(false)
  })

  it('keeps the scroll between the top and the lowest row, and falls back on a NaN', () => {
    const max = maxScrollY(400, layout)
    expect(max).toBe(128 * 16 - (400 - 28))
    expect(clampScrollY(-5, 400, layout)).toBe(0)
    expect(clampScrollY(1e9, 400, layout)).toBe(max)
    expect(clampScrollY(Number.NaN, 400, layout)).toBe(0)
    // A surface taller than all the rows has nothing to scroll.
    expect(maxScrollY(5000, layout)).toBe(0)
  })

  it('opens around the notes, or around middle C when the clip is empty', () => {
    const empty = initialScrollY([], 400, layout)
    expect(pitchAt(28 + (400 - 28) / 2, { ...layout, scrollY: empty })).toBeGreaterThanOrEqual(59)
    expect(pitchAt(28 + (400 - 28) / 2, { ...layout, scrollY: empty })).toBeLessThanOrEqual(61)
    const high = initialScrollY([note(0, 100), note(0, 104)], 400, layout)
    expect(pitchAt(28 + (400 - 28) / 2, { ...layout, scrollY: high })).toBeGreaterThanOrEqual(101)
  })
})

describe('hit testing', () => {
  const notes = [note(0, 60, 960), note(960, 62, 960)]
  const at = (n: Note) => {
    const y = pitchTop(n.pitch, layout) + 4
    return { x: n.start * view.pxPerTick + 20, y }
  }

  it('finds the note under the pointer, on its body or on its end', () => {
    expect(hitNote(notes, at(notes[0]!).x, at(notes[0]!).y, view, layout)).toEqual({ index: 0, zone: 'body' })
    expect(hitNote(notes, 960 * 0.1 - 2, pitchTop(60, layout) + 4, view, layout)).toEqual({ index: 0, zone: 'end' })
    expect(hitNote(notes, at(notes[1]!).x, at(notes[1]!).y, view, layout)).toEqual({ index: 1, zone: 'body' })
  })

  it('misses a note on another row, past its end, or under the ruler', () => {
    expect(hitNote(notes, 20, pitchTop(61, layout) + 4, view, layout)).toBeNull()
    expect(hitNote(notes, 960 * 0.1 + 5, pitchTop(60, layout) + 4, view, layout)).toBeNull()
    expect(hitNote(notes, 20, 10, view, layout)).toBeNull()
  })

  it('takes the last of two notes that overlap, as it is drawn on top', () => {
    const stacked = [note(0, 60, 960), note(480, 60, 960, 90)]
    expect(hitNote(stacked, 60, pitchTop(60, layout) + 4, view, layout)?.index).toBe(1)
  })

  it('gives a very short note a body to grab and no end zone', () => {
    const tiny = [note(0, 60, 10)]
    expect(hitNote(tiny, 1, pitchTop(60, layout) + 4, view, layout)).toEqual({ index: 0, zone: 'body' })
  })

  it('selects the notes a rectangle touches', () => {
    const rect = rectBetween(150, pitchTop(62, layout) - 2, 5, pitchTop(60, layout) + 4)
    expect(notesInRect(notes, rect, view, layout).sort()).toEqual(notes.map(noteKey).sort())
    expect(notesInRect(notes, rectBetween(0, 0, 10, 10), view, layout)).toEqual([])
    expect(notesInRect(notes, rectBetween(5, pitchTop(60, layout), 10, pitchTop(60, layout) + 3), view, layout)).toEqual([noteKey(notes[0]!)])
  })
})

describe('note keys', () => {
  it('tells two notes apart by any field, defaults being the same as absent', () => {
    expect(noteKey(note(0, 60))).toBe(noteKey({ ...note(0, 60), cents: 0, chance: 100, repeat: 1 }))
    expect(noteKey(note(0, 60))).not.toBe(noteKey(note(0, 60, 480, 99)))
    expect(noteKey(note(0, 60))).not.toBe(noteKey({ ...note(0, 60), cents: 5 }))
  })
})

describe('move and resize', () => {
  const group = [note(960, 60, 480), note(1440, 64, 480)]

  it('moves a group by a tick and a pitch delta', () => {
    const changes = moveNotes(group, 480, -2, 3840)
    expect(changes.map((c) => c.to)).toEqual([note(1440, 58, 480), note(1920, 62, 480)])
    expect(changes.map((c) => c.from)).toEqual(group)
  })

  it('cuts the delta so that the whole group stays in the clip and on the keyboard', () => {
    expect(moveNotes(group, -5000, 0, 3840).map((c) => c.to.start)).toEqual([0, 480])
    expect(moveNotes(group, 99999, 0, 3840).map((c) => c.to.start + c.to.length)).toEqual([3360, 3840])
    expect(moveNotes(group, 0, 99, 3840).map((c) => c.to.pitch)).toEqual([123, 127])
    expect(moveNotes(group, 0, -99, 3840).map((c) => c.to.pitch)).toEqual([0, 4])
    expect(moveNotes(group, Number.NaN, Number.NaN, 3840).map((c) => c.to)).toEqual(group)
    expect(moveNotes([], 10, 10, 3840)).toEqual([])
  })

  it('never makes two notes the same by cutting the delta: the group moves as one', () => {
    const top = [note(0, 126), note(0, 127)]
    const moved = moveNotes(top, 0, 5, 3840).map((c) => c.to)
    expect(moved).toEqual(top)
    const low = [note(0, 0), note(0, 1)]
    expect(new Set(moveNotes(low, 0, -5, 3840).map((c) => noteKey(c.to))).size).toBe(2)
  })

  it('keeps a note that fills the clip where it is', () => {
    const whole = [note(0, 60, 3840)]
    expect(moveNotes(whole, 480, 0, 3840)[0]!.to.start).toBe(0)
  })

  it('resizes a group, never below one tick nor past the clip', () => {
    expect(resizeNotes(group, 240, 3840).map((c) => c.to.length)).toEqual([720, 720])
    expect(resizeNotes(group, -9999, 3840).map((c) => c.to.length)).toEqual([1, 1])
    // The group ends at 1920: it can grow by 1920 at most, the later note then reaching the clip's end.
    expect(resizeNotes(group, 9999, 3840).map((c) => c.to.length)).toEqual([2400, 2400])
  })

  it('puts the default velocity back on the chosen notes, as one change, and nothing when they have it', () => {
    const all = [note(0, 60, 480, 40), note(480, 62, 480, 100), note(960, 64, 480, 127)]
    expect(resetVelocities(all, all)?.map((c) => c.to.velocity)).toEqual([100, 100])
    expect(resetVelocities(all, [all[1]!])).toEqual([])
    expect(resetVelocities(all, [])).toEqual([])
    // Two stacked notes that differ only in velocity would become one: refused as a whole.
    const stacked = [note(0, 60, 480, 80), note(0, 60, 480, 110)]
    expect(resetVelocities(stacked, stacked)).toBeNull()
    expect(resetVelocities(stacked, [stacked[0]!])?.length).toBe(1)
  })

  it('shifts velocities together inside 1 to 127', () => {
    const pair = [note(0, 60, 480, 120), note(0, 61, 480, 40)]
    expect(shiftVelocity(pair, 5).map((c) => c.to.velocity)).toEqual([125, 45])
    // The shift is cut for the whole group, so two notes never land on the same velocity.
    expect(shiftVelocity(pair, 20).map((c) => c.to.velocity)).toEqual([127, 47])
    expect(shiftVelocity(pair, -100).map((c) => c.to.velocity)).toEqual([81, 1])
    const close = [note(0, 60, 480, 120), note(0, 60, 480, 125)]
    expect(acceptable(close, shiftVelocity(close, 30))).not.toBeNull()
    expect(shiftVelocity([], 5)).toEqual([])
  })

  it('reads a velocity from a position, the top being the loudest', () => {
    expect(velocityAt(0, 0, 100)).toBe(127)
    expect(velocityAt(100, 0, 100)).toBe(1)
    expect(velocityAt(50, 0, 100)).toBe(64)
    expect(velocityAt(-40, 0, 100)).toBe(127)
    expect(velocityAt(Number.NaN, 0, 100)).toBe(1)
  })

  it('gives the lane a position for the shared drag, and the velocity back from it', () => {
    expect(velocityPosition(0, 0, 100)).toBe(1)
    expect(velocityPosition(100, 0, 100)).toBe(0)
    expect(velocityPosition(-40, 0, 100)).toBe(1)
    expect(velocityPosition(25, 0, 100)).toBeCloseTo(0.75, 10)
    expect(velocityOf(0)).toBe(1)
    expect(velocityOf(1)).toBe(127)
    expect(velocityOf(Number.NaN)).toBe(1)
    // The default velocity is a position the magnet can hold, and comes back exactly.
    expect(velocityOf(VELOCITY_HOME)).toBe(DEFAULT_VELOCITY)
  })
})

describe('what the clip would refuse', () => {
  const all = [note(0, 60), note(480, 60), note(960, 62)]

  it('drops the changes that change nothing', () => {
    expect(acceptable(all, [{ from: all[0]!, to: { ...all[0]! } }])).toEqual([])
  })

  it('accepts a shift of a chain onto the places the group leaves', () => {
    const changes = [
      { from: all[0]!, to: note(480, 60) },
      { from: all[1]!, to: note(960, 60) },
    ]
    expect(acceptable(all, changes)).toEqual(changes)
  })

  it('refuses a move onto a note that stays, and two notes that arrive on the same value', () => {
    expect(acceptable(all, [{ from: all[0]!, to: note(480, 60) }])).toBeNull()
    expect(
      acceptable(all, [
        { from: all[0]!, to: note(1920, 60) },
        { from: all[2]!, to: note(1920, 60) },
      ]),
    ).toBeNull()
  })
})

describe('drawing a note', () => {
  it('starts at the cell of the click and takes the last length when the pointer did not move', () => {
    expect(drawnNote(1000, null, 60, 100, 480, 960, 3840)).toEqual(note(960, 60, 960, 100))
    expect(drawnNote(1000, 1000, 60, 100, 480, 960, 3840)).toEqual(note(960, 60, 960, 100))
  })

  it('ends at the snapped pointer when dragged, at least one step', () => {
    expect(drawnNote(960, 2400, 61, 90, 480, 960, 3840)).toEqual(note(960, 61, 1440, 90))
    expect(drawnNote(960, 1100, 61, 90, 480, 960, 3840)).toEqual(note(960, 61, 960, 90))
  })

  it('stays inside the clip and refuses a press past its end', () => {
    expect(drawnNote(3600, null, 60, 100, 480, 960, 3840)).toEqual(note(3360, 60, 480, 100))
    expect(drawnNote(3840, null, 60, 100, 480, 960, 3840)).toBeNull()
    expect(drawnNote(0, 99999, 60, 100, 480, 960, 3840)?.length).toBe(3840)
  })

  it('works with the snap off, and clamps pitch and velocity', () => {
    expect(drawnNote(1001.4, null, 200, 500, null, 100, 3840)).toEqual({ start: 1001, pitch: 127, length: 100, velocity: 127 })
    expect(floorTick(-5, 480)).toBe(0)
    expect(floorTick(Number.NaN, 480)).toBe(0)
  })
})

describe('changes that outlive their clip', () => {
  const a = note(0, 60)
  const b = note(960, 62)

  it('still apply while every note they replace is in the clip and every note they bring fits it', () => {
    expect(stillApplies([a, b], [{ from: a, to: note(480, 60) }, { from: b, to: note(1440, 62) }], 3840)).toBe(true)
  })

  it('stop applying when a replaced note is gone, even one that was not the grabbed one', () => {
    expect(stillApplies([a], [{ from: a, to: note(480, 60) }, { from: b, to: note(1440, 62) }], 3840)).toBe(false)
  })

  it('stop applying when the clip became shorter than the note they bring', () => {
    expect(stillApplies([a], [{ from: a, to: note(3000, 60) }], 3840)).toBe(true)
    expect(stillApplies([a], [{ from: a, to: note(3000, 60) }], 3000)).toBe(false)
  })

  it('knows what the clip accepts of a note', () => {
    expect(fits(note(0, 60, 3840), 3840)).toBe(true)
    expect(fits(note(0, 60, 3841), 3840)).toBe(false)
    expect(fits(note(-1, 60), 3840)).toBe(false)
    expect(fits(note(0, 128), 3840)).toBe(false)
    expect(fits(note(0, 60, 0), 3840)).toBe(false)
    expect(fits(note(0, 60, 480, 0), 3840)).toBe(false)
  })
})

describe('velocity lane', () => {
  it('finds the bar near the pointer, the last first', () => {
    const notes = [note(0, 60), note(960, 62), note(960, 64)]
    expect(hitVelocity(notes, 97, view)).toBe(2)
    expect(hitVelocity(notes, 50, view)).toBeNull()
    expect(hitVelocity(notes, 1, view)).toBe(0)
  })
})
