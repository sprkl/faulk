import { describe, expect, it } from 'vitest'
import type { Clip, Track } from '../../lib/generated/protocol'
import { clipLabel, clipOrigin, editedClip, ghostNotes, midiClips } from './clips'

const midi = (name = ''): Clip => ({ name, content: { midi: { length: 3840, notes: [{ start: 0, pitch: 60, length: 480, velocity: 100 }] } } })
const audio = (): Clip => ({ name: 'a', content: { audio: { frames: 10, media: 'm', source_start: 0 } } }) as unknown as Clip
const track = (clips: Record<string, Clip>, placed: Record<string, [string, number]> = {}): Track =>
  ({
    clips,
    placements: Object.fromEntries(Object.entries(placed).map(([id, [clip, start]]) => [id, { clip, start, span: { start: 0, length: 3840 } }])),
  }) as unknown as Track

describe('the clips the roll offers', () => {
  it('lists MIDI clips only, the earliest placed first, then the unplaced by identifier', () => {
    const t = track({ c: midi(), b: midi(), a: midi(), z: audio() }, { p1: ['b', 7680], p2: ['c', 0], p3: ['c', 100] })
    expect(midiClips(t).map((c) => c.id)).toEqual(['c', 'b', 'a'])
  })

  it('reads length, loop and notes of the clip', () => {
    const [only] = midiClips(track({ a: midi() }))
    expect(only).toMatchObject({ id: 'a', length: 3840, looping: null })
    expect(only!.notes).toHaveLength(1)
  })

  it('opens on the first clip, keeps a choice that still exists and drops one that is gone', () => {
    const clips = midiClips(track({ a: midi(), b: midi() }))
    expect(editedClip(clips, null)?.id).toBe('a')
    expect(editedClip(clips, 'b')?.id).toBe('b')
    expect(editedClip(clips, 'gone')?.id).toBe('a')
    expect(editedClip([], 'a')).toBeNull()
  })

  it('names a clip by the name the project stores, whatever its place in the list', () => {
    const [a, b, c] = midiClips(track({ a: midi('Clip 7'), b: midi('Bass'), c: midi('  ') }))
    expect(clipLabel(a!, 'Clip')).toBe('Clip 7')
    expect(clipLabel(b!, 'Clip')).toBe('Bass')
    expect(clipLabel(c!, 'Clip')).toBe('Clip')
  })

  it('says where a clip comes from: its grid cell, the arrangement, or both', () => {
    const inGrid = (name: string, slot: string): Clip => ({ ...midi(name), slot }) as Clip
    const t = track({ g: inGrid('G', 's2'), p: midi('P'), both: inGrid('B', 's1'), none: midi('N') }, { p1: ['p', 0], p2: ['both', 0] })
    const by = Object.fromEntries(midiClips(t).map((c) => [c.id, c]))
    const labels = { grid: (id: string) => `Grid · Scene ${id.slice(1)}`, arrangement: (start: number) => `Arrangement · ${start}` }
    expect(clipOrigin(by.g!, labels)).toBe('Grid · Scene 2')
    expect(clipOrigin(by.p!, labels)).toBe('Arrangement · 0')
    expect(clipOrigin(by.both!, labels)).toBe('Grid · Scene 1, Arrangement · 0')
    expect(clipOrigin(by.none!, labels)).toBe('')
  })
})

describe('ghost notes of the other tracks (#470)', () => {
  const placed = (clips: Record<string, Clip>, placements: Record<string, [string, number, number, number]>, kind = 'midi'): Track =>
    ({
      kind,
      clips,
      placements: Object.fromEntries(
        Object.entries(placements).map(([id, [clip, start, offset, length]]) => [id, { clip, start, span: { ticks: { offset, length } } }]),
      ),
    }) as unknown as Track
  const note = (start: number, pitch = 60) => ({ start, pitch, length: 100, velocity: 100 })
  const clipOf = (notes: ReturnType<typeof note>[], slot?: string): Clip => ({ name: '', slot, content: { midi: { length: 3840, notes } } }) as unknown as Clip

  it('shifts the notes of overlapping placements into the time of the edited clip and drops the rest', () => {
    const mine = placed({ a: clipOf([note(0)]) }, { p: ['a', 3840, 0, 3840] })
    const other = placed({ b: clipOf([note(0, 40), note(3000, 41)]) }, { q: ['b', 3840 + 480, 0, 2000], r: ['b', 0, 0, 3840] })
    const edited = midiClips(mine)[0]!
    const ghosts = ghostNotes(
      [
        { id: 'me', track: mine },
        { id: 'o', track: other },
      ],
      'me',
      edited,
      mine,
    )
    expect(ghosts).toEqual([{ start: 480, length: 100, pitch: 40 }])
  })

  it('never ghosts the edited track or an audio track, and uses the scene for a clip that is only in the grid', () => {
    const mine = placed({ a: clipOf([note(0)], 's1') }, {})
    const other = placed({ b: clipOf([note(10, 50)], 's1'), c: clipOf([note(20, 51)], 's2') }, {})
    const audio = placed({}, {}, 'audio')
    const edited = midiClips(mine)[0]!
    const tracks = [
      { id: 'me', track: mine },
      { id: 'o', track: other },
      { id: 'au', track: audio },
    ]
    expect(ghostNotes(tracks, 'me', edited, mine)).toEqual([{ start: 10, length: 100, pitch: 50 }])
    expect(ghostNotes([tracks[0]!], 'me', edited, mine)).toEqual([])
    expect(ghostNotes(tracks, 'me', null, mine)).toEqual([])
  })
})
