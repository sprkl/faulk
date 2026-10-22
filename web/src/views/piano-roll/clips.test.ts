import { describe, expect, it } from 'vitest'
import type { Clip, Track } from '../../lib/generated/protocol'
import { clipLabel, editedClip, midiClips } from './clips'

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

  it('names a clip by its own name or by its place', () => {
    const [a, b] = midiClips(track({ a: midi('Bass'), b: midi('  ') }))
    const fallback = (n: number) => `Clip ${n}`
    expect(clipLabel(a!, 0, fallback)).toBe('Bass')
    expect(clipLabel(b!, 1, fallback)).toBe('Clip 2')
  })
})
