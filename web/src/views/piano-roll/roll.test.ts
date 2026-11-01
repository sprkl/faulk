import { describe, expect, it } from 'vitest'
import type { Note } from '../../lib/generated/protocol'
import {
  acceptable,
  clampScrollY,
  defaultLength,
  drawnNote,
  fits,
  floorTick,
  hitNote,
  hitVelocity,
  velocityTarget,
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
  quantizedStart,
  quantizeChanges,
  transposeChanges,
  toBoard,
  fromBoard,
  endOf,
  lengthFor,
  duplicateOffset,
  drawStep,
  MAX_BARS,
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

describe('the keys of a kit', () => {
  it('opens on the sounds of the kit, and on the notes too when some are outside', () => {
    const kit = initialScrollY([], 400, layout, [36, 38, 39, 42, 46])
    const middle = pitchAt(28 + (400 - 28) / 2, { ...layout, scrollY: kit })
    expect(middle).toBeGreaterThanOrEqual(40)
    expect(middle).toBeLessThanOrEqual(42)
    const wide = initialScrollY([note(0, 80)], 400, layout, [36, 46])
    expect(wide).toBeLessThan(kit)
  })
})

describe('the default length of a drawn note', () => {
  it('is the last drawn or resized length, else the snap step, else a sixteenth', () => {
    expect(defaultLength(null, 240)).toBe(240)
    expect(defaultLength(null, null)).toBe(240)
    expect(defaultLength(120, 960)).toBe(120)
    expect(defaultLength(null, 960)).toBe(960)
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

  it('leaves the cell right after a note free, so that a click there draws', () => {
    const one = [note(0, 60, 240)]
    const right = 240 * view.pxPerTick + 20 - 20
    expect(hitNote(one, right, pitchTop(60, layout) + 4, view, layout)).toBeNull()
    expect(hitNote(one, right - 0.5, pitchTop(60, layout) + 4, view, layout)).not.toBeNull()
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
    expect(
      stillApplies(
        [a, b],
        [
          { from: a, to: note(480, 60) },
          { from: b, to: note(1440, 62) },
        ],
        3840,
      ),
    ).toBe(true)
  })

  it('stop applying when a replaced note is gone, even one that was not the grabbed one', () => {
    expect(
      stillApplies(
        [a],
        [
          { from: a, to: note(480, 60) },
          { from: b, to: note(1440, 62) },
        ],
        3840,
      ),
    ).toBe(false)
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

describe('quantize, transpose, copy and paste (PN-2)', () => {
  it('quantizes a start like the command: nearest line, halfway to the later one, by a share of the distance', () => {
    // The same table as `quantizing_a_start_moves_a_share_of_the_way_to_the_nearest_line` in crates/commands/src/clip.rs.
    expect(quantizedStart(150, 240, 100, 10, 3840)).toBe(240)
    expect(quantizedStart(119, 240, 100, 10, 3840)).toBe(0)
    expect(quantizedStart(120, 240, 100, 10, 3840)).toBe(240)
    expect(quantizedStart(150, 240, 0, 10, 3840)).toBe(150)
    expect(quantizedStart(150, 240, 50, 10, 3840)).toBe(195)
    expect(quantizedStart(150, 240, 1, 10, 3840)).toBe(151)
    expect(quantizedStart(20, 240, 1, 10, 3840)).toBe(20)
    expect(quantizedStart(30, 240, 25, 10, 3840)).toBe(22)
    expect(quantizedStart(210, 240, 25, 10, 3840)).toBe(218)
    expect(quantizedStart(77, 1, 100, 10, 3840)).toBe(77)
    expect(quantizedStart(77, 100_000, 100, 10, 3840)).toBe(0)
  })

  it('keeps a note that would cross the clip end inside it, and a note as long as the clip at 0', () => {
    expect(quantizedStart(3000, 1920, 100, 480, 3840)).toBe(3360)
    expect(quantizedStart(3360, 1920, 100, 480, 3840)).toBe(3360)
    expect(quantizedStart(0, 1920, 100, 3840, 3840)).toBe(0)
    expect(quantizedStart(-10, 240, 100, 10, 3840)).toBe(0)
  })

  it('lists the changes of a quantize', () => {
    const changes = quantizeChanges([note(100, 60, 50), note(240, 62, 50)], 240, 100, 3840)
    expect(changes.map((c) => c.to.start)).toEqual([0, 240])
  })

  it('transposes, and gives nothing when a note would leave the keyboard', () => {
    expect(transposeChanges([note(0, 60), note(0, 62)], 12)?.map((c) => c.to.pitch)).toEqual([72, 74])
    expect(transposeChanges([note(0, 60)], 0)).toEqual([])
    expect(transposeChanges([], 1)).toEqual([])
    expect(transposeChanges([note(0, 60), note(0, 127)], 1)).toBeNull()
    expect(transposeChanges([note(0, 0)], -1)).toBeNull()
    expect(transposeChanges([note(0, 120)], 7)?.[0]?.to.pitch).toBe(127)
  })

  it('puts notes aside relative to the earliest and puts them back anywhere', () => {
    const board = toBoard([note(960, 60), note(480, 62)])
    expect(board.map((n) => n.start)).toEqual([480, 0])
    expect(fromBoard(board, 100).map((n) => n.start)).toEqual([580, 100])
    expect(toBoard([])).toEqual([])
    expect(fromBoard(board, -50).map((n) => n.start)).toEqual([480, 0])
  })

  it('grows a clip to the next whole bar, never past the longest clip', () => {
    expect(endOf([note(0, 60, 480), note(960, 60, 480)])).toBe(1440)
    expect(endOf([])).toBe(0)
    expect(lengthFor(3840, 3840, 3840)).toBe(3840)
    expect(lengthFor(3841, 3840, 3840)).toBe(7680)
    expect(lengthFor(MAX_BARS * 3840, 3840, 3840)).toBe(MAX_BARS * 3840)
    expect(lengthFor(MAX_BARS * 3840 + 1, 3840, 3840)).toBeNull()
  })

  it('duplicates by whole bars, counted from the bar of the first note, at least one bar', () => {
    const bar = 3840
    // One bar or less: the next bar, whatever the position in the bar.
    expect(duplicateOffset([note(480, 60, 480), note(1440, 62, 480)], bar)).toBe(bar)
    expect(duplicateOffset([note(3000, 60, 840)], bar)).toBe(bar)
    expect(duplicateOffset([note(0, 60, bar)], bar)).toBe(bar)
    expect(duplicateOffset([note(bar + 100, 60, 10)], bar)).toBe(bar)
    // Across a bar line the span is measured from the first bar: two bars.
    expect(duplicateOffset([note(3800, 60, 480), note(bar + 100, 62, 100)], bar)).toBe(2 * bar)
    expect(duplicateOffset([note(0, 60, bar + 1)], bar)).toBe(2 * bar)
    // A selection of two bars goes two bars later; another meter changes the bar.
    expect(duplicateOffset([note(0, 60, 2 * bar)], bar)).toBe(2 * bar)
    expect(duplicateOffset([note(0, 60, 2000)], 2880)).toBe(2880)
    expect(duplicateOffset([], bar)).toBe(0)
  })

  it('draws on a step that Auto never takes below a sixteenth, and keeps a step the musician picked', () => {
    // Auto at a high zoom is finer than a sixteenth: drawing stops at a sixteenth.
    expect(drawStep(true, 120)).toBe(240)
    expect(drawStep(true, 60)).toBe(240)
    // A coarser Auto step (zoomed out) is kept.
    expect(drawStep(true, 480)).toBe(480)
    expect(drawStep(true, 3840)).toBe(3840)
    // A picked step is used as is, and no snapping stays off.
    expect(drawStep(false, 120)).toBe(120)
    expect(drawStep(false, null)).toBeNull()
    expect(drawStep(true, null)).toBeNull()
    // A click puts a note on the step, one step long; a drag end is already rounded to the step (700 gives 720, 130 gives 240).
    const step = drawStep(true, 120)
    expect(drawnNote(100, null, 60, 100, step, defaultLength(null, step), 3840)).toEqual(note(0, 60, 240, 100))
    expect(drawnNote(100, 720, 60, 100, step, defaultLength(null, step), 3840)).toEqual(note(0, 60, 720, 100))
    expect(drawnNote(100, 240, 60, 100, step, defaultLength(null, step), 3840)).toEqual(note(0, 60, 240, 100))
  })
})

describe('velocity lane', () => {
  it('finds the bar near the pointer, the last first', () => {
    const notes = [note(0, 60), note(960, 62), note(960, 64)]
    expect(hitVelocity(notes, 97, view)).toBe(2)
    expect(hitVelocity(notes, 50, view)).toBeNull()
    expect(hitVelocity(notes, 1, view)).toBe(0)
  })

  it('picks, among the notes of one step, the selected one, else the highest', () => {
    const notes = [note(960, 42), note(960, 46), note(0, 60)]
    expect(velocityTarget(notes, 97, view, new Set())).toBe(1)
    expect(velocityTarget(notes, 97, view, new Set([noteKey(notes[0]!)]))).toBe(0)
    expect(velocityTarget(notes, 97, view, new Set([noteKey(notes[0]!), noteKey(notes[1]!)]))).toBe(1)
    // A selected note on another step does not matter.
    expect(velocityTarget(notes, 97, view, new Set([noteKey(notes[2]!)]))).toBe(1)
    expect(velocityTarget(notes, 50, view, new Set())).toBeNull()
  })
})
