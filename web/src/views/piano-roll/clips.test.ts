import { describe, expect, it } from 'vitest'
import type { Clip, Track } from '../../lib/generated/protocol'
import { clipLabel, clipOrigin, editedClip, midiClips } from './clips'

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
    const scene = (id: string) => `Scene ${id.slice(1)}`
    expect(clipOrigin(by.g!, scene, 'Arrangement')).toBe('Scene 2')
    expect(clipOrigin(by.p!, scene, 'Arrangement')).toBe('Arrangement')
    expect(clipOrigin(by.both!, scene, 'Arrangement')).toBe('Scene 1, Arrangement')
    expect(clipOrigin(by.none!, scene, 'Arrangement')).toBe('')
  })
})
