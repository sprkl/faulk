import { describe, expect, it } from 'vitest'
import type { Project } from './generated/protocol'
import { freeScene, gridCommand, newClipAt, placeCommand, poolNames, poolOf } from './clip-pool'

const midi = (name: string, slot: string | null = null) => ({
  name,
  color: null,
  slot,
  content: { midi: { length: 3840, looping: null, notes: [] } },
})
const placed = (clip: string, start: number) => ({ clip, start, span: { ticks: { length: 3840, offset: 0 } } })

const project = {
  scenes: { s1: { name: 'Scene 1', order: 'a0' }, s2: { name: 'Scene 2', order: 'a1' } },
  tracks: {
    t: {
      clips: { grid: midi('Riff', 's1'), arr: midi('Bass 1'), loose: midi('Bass 2'), both: midi('Lead', 's2') },
      placements: { p1: placed('arr', 7680), p0: placed('arr', 0), p2: placed('both', 0) },
    },
  },
} as unknown as Pick<Project, 'tracks' | 'scenes'>

describe('the clip pool (#473)', () => {
  it('lists every clip of the track with its uses', () => {
    const pool = poolOf(project, 't')
    expect(pool.map((c) => [c.id, c.uses])).toEqual([
      ['arr', 2],
      ['loose', 0],
      ['both', 2],
      ['grid', 1],
    ])
    expect(pool[0]!.placements.map((p) => p.id)).toEqual(['p0', 'p1'])
    expect(poolOf(project, 'nope')).toEqual([])
  })

  it('places a clip used nowhere itself, and copies a used one unless linked (founder decision 11)', () => {
    const [arr, loose, both, grid] = poolOf(project, 't')
    expect(placeCommand(loose!, 960, false)).toEqual({ type: 'arrangement.place', clip: 'loose', start: 960 })
    // A start inside the source placement: the copy goes right after it, never over it.
    expect(placeCommand(arr!, 960, false)).toEqual({ type: 'arrangement.duplicate', placements: ['p0'], linked: false, by: 3840 })
    expect(placeCommand(arr!, 19200, false)).toEqual({ type: 'arrangement.duplicate', placements: ['p0'], linked: false, by: 19200 })
    expect(placeCommand(arr!, 960, true)).toEqual({ type: 'arrangement.place', clip: 'arr', start: 960 })
    // At its own start, a copy goes right after its source, never onto it.
    expect(placeCommand(arr!, 0, false)).toEqual({ type: 'arrangement.duplicate', placements: ['p0'], linked: false, by: 3840 })
    expect(placeCommand(grid!, 960, false)).toEqual({ type: 'arrangement.place_from_slot', clip: 'grid', start: 960, linked: false })
    expect(placeCommand(both!, 960, true)).toEqual({ type: 'arrangement.place_from_slot', clip: 'both', start: 960, linked: true })
  })

  it('sends the refusal to carve nothing when the buttons ask for it, for every command it picks (#805)', () => {
    const [arr, loose, both, grid] = poolOf(project, 't')
    expect(placeCommand(loose!, 960, false, 'refuse')).toEqual({ type: 'arrangement.place', clip: 'loose', start: 960, if_occupied: 'refuse' })
    expect(placeCommand(arr!, 19200, false, 'refuse')).toEqual({
      type: 'arrangement.duplicate',
      placements: ['p0'],
      linked: false,
      by: 19200,
      if_occupied: 'refuse',
    })
    expect(placeCommand(arr!, 960, true, 'refuse')).toEqual({ type: 'arrangement.place', clip: 'arr', start: 960, if_occupied: 'refuse' })
    expect(placeCommand(grid!, 960, false, 'refuse')).toMatchObject({ type: 'arrangement.place_from_slot', if_occupied: 'refuse' })
    expect(placeCommand(both!, 960, true, 'refuse')).toMatchObject({ type: 'arrangement.place_from_slot', if_occupied: 'refuse' })
  })

  it('puts a clip used nowhere in a slot itself, and copies a used one unless linked', () => {
    const [arr, loose, , grid] = poolOf(project, 't')
    expect(gridCommand(loose!, 's9', false)).toEqual({ type: 'clip.move', clip: 'loose', slot: 's9', track: null })
    expect(gridCommand(arr!, 's9', false)).toEqual({ type: 'clip.duplicate', clip: 'arr', slot: 's9', track: null })
    expect(gridCommand(arr!, 's9', true)).toEqual({ type: 'clip.move', clip: 'arr', slot: 's9', track: null })
    // A clip in the grid already is copied (a clip sits in one slot at most).
    expect(gridCommand(grid!, 's9', true)).toEqual({ type: 'clip.duplicate', clip: 'grid', slot: 's9', track: null })
  })

  it('tells two clips of one name apart by a number (#624)', () => {
    const twins = {
      tracks: { t: { clips: { b: midi('Riff'), a: midi('Riff'), c: midi('Bass'), d: midi('') }, placements: {} } },
    } as unknown as Pick<Project, 'tracks'>
    expect(poolNames(poolOf(twins, 't'), 'Audio clip')).toEqual({ c: 'Bass', a: 'Riff (1)', b: 'Riff (2)', d: 'Audio clip' })
  })

  it('puts an audio clip in a slot or the arrangement like a MIDI one (AU-6): itself when unused, else a copy', () => {
    const audio = (slot: string | null) => ({
      name: 'Loop',
      color: null,
      slot,
      content: { audio: { media: { hash: 'a', sample_rate: 48000, channels: 2, frames: 48000, name: 'loop.wav' } } },
    })
    const withAudio = {
      scenes: project.scenes,
      tracks: { a: { clips: { loose: audio(null), used: audio('s1') }, placements: {} } },
    } as unknown as Pick<Project, 'tracks' | 'scenes'>
    const [loose, used] = poolOf(withAudio, 'a')
    expect(loose!.midi).toBe(false)
    expect(gridCommand(loose!, 's2', false)).toEqual({ type: 'clip.move', clip: 'loose', slot: 's2', track: null })
    expect(gridCommand(used!, 's2', false)).toMatchObject({ type: 'clip.duplicate', clip: 'used', slot: 's2' })
    expect(placeCommand(loose!, 3840, false)).toEqual({ type: 'arrangement.place', clip: 'loose', start: 3840 })
    expect(placeCommand(used!, 3840, false)).toMatchObject({ type: 'arrangement.place_from_slot', clip: 'used', start: 3840, linked: false })
  })

  it('measures an audio placement in ticks at the project tempo (AU-6), so a copy lands after it', () => {
    const clip = {
      name: 'Loop',
      color: null,
      slot: null,
      content: { audio: { media: { hash: 'a', sample_rate: 48000, channels: 2, frames: 96000, name: 'loop.wav' }, start: 0, frames: 96000 } },
    }
    const placed = {
      tempo: [{ tick: 0, bpm: 120 }],
      tracks: { a: { clips: { c: clip }, placements: { p: { clip: 'c', start: 0, span: { frames: { length: 48000, offset: 0 } } } } } },
    } as unknown as Pick<Project, 'tracks' | 'tempo'>
    const [entry] = poolOf(placed, 'a')
    // One second at 120 bpm is 1920 ticks.
    expect(entry!.placements).toEqual([{ id: 'p', start: 0, length: 1920 }])
    // A copy at the playhead inside the placement goes right after it.
    expect(placeCommand(entry!, 960, false)).toMatchObject({ type: 'arrangement.duplicate', placements: ['p'], linked: false, by: 1920 })
  })

  it('finds the first free slot of the track in scene order', () => {
    expect(freeScene(project, 't')).toBeNull()
    const more = { ...project, scenes: { ...project.scenes, s0: { name: 'Scene 0', order: 'Zz' }, s3: { name: 'Scene 3', order: 'a2' } } }
    expect(freeScene(more, 't')).toBe('s0')
  })
})

describe('a new empty clip at a bar (#852)', () => {
  const tracks = {
    tracks: {
      m: { kind: 'midi', clips: { a: midi('A') }, placements: { p: placed('a', 7680) } },
      empty: { kind: 'midi', clips: {}, placements: {} },
      au: { kind: 'audio', clips: {}, placements: {} },
    },
  } as unknown as Pick<Project, 'tracks'>

  it('creates the clip on a free bar, an empty track included', () => {
    expect(newClipAt(tracks, 'empty', 0, 3840)).toEqual({ command: { type: 'clip.create', track: 'empty', at: { at: 'arrangement', start: 0 }, length: 3840 } })
    expect(newClipAt(tracks, 'm', 3840, 3840)).toHaveProperty('command')
    // Touching is not overlapping.
    expect(newClipAt(tracks, 'm', 11520, 3840)).toHaveProperty('command')
  })

  it('never overwrites a placement', () => {
    for (const start of [7680, 7680 + 960, 7680 - 960]) expect(newClipAt(tracks, 'm', start, 3840), String(start)).toEqual({ refused: 'noRoom' })
  })

  it('refuses an audio track and an unknown one', () => {
    expect(newClipAt(tracks, 'au', 0, 3840)).toEqual({ refused: 'notMidi' })
    expect(newClipAt(tracks, 'nope', 0, 3840)).toEqual({ refused: 'notMidi' })
  })
})
