import { render } from 'svelte/server'
import { describe, expect, it } from 'vitest'
import type { Clip, ProjectView } from '../../lib/generated/protocol'
import { createStores, storesContext } from '../../lib/stores'
import PianoRollView from './PianoRollView.svelte'
import { afterChanges, noteKey, toggled } from './roll'

const midiClip = (name: string): Clip => ({ name, content: { midi: { length: 7680, notes: [{ start: 0, pitch: 60, length: 480, velocity: 100 }] } } })

function view(kind: 'midi' | 'audio', clips: Record<string, Clip> = {}): ProjectView {
  return {
    project: { meter: [{ tick: 0, signature: { numerator: 4, denominator: 4 } }], tracks: { t1: { kind, order: 'a', name: 'T', clips, placements: {}, devices: {} } } },
    session: { selected_track: 't1' },
  } as unknown as ProjectView
}

function body(project: ProjectView | null, lang: 'en' | 'fr' = 'en') {
  const stores = createStores('play', 'piano-roll')
  stores.ui.lang = lang
  if (project) stores.project.show(project)
  return render(PianoRollView, { context: storesContext(stores) }).body
}

describe('what the piano roll shows before it has a clip to edit', () => {
  it('asks for a MIDI track when none is selected, in the page language', () => {
    expect(body(null)).toContain('Select a MIDI track to edit its notes.')
    expect(body(null, 'fr')).toContain('Sélectionne une piste MIDI pour modifier ses notes.')
  })

  it('says an audio track has no notes', () => {
    expect(body(view('audio'))).toContain('An audio track has no notes to edit.')
  })

  it('offers to create a clip on a MIDI track that has none', () => {
    const html = body(view('midi'))
    expect(html).toContain('data-testid="roll-create-clip"')
    expect(html).toContain('This track has no clip yet.')
    expect(html).not.toContain('data-testid="piano-roll"')
  })

  it('opens on the clip of the track, with its notes counted and its length shown in bars', () => {
    const html = body(view('midi', { a: midiClip('Bass') }))
    expect(html).toContain('data-testid="piano-roll"')
    expect(html).toContain('data-note-count="1"')
    expect(html).toContain('data-clip-length="7680"')
    expect(html).toContain('Bass')
    expect(html).toContain('value="2"')
    // One clip: its name, not a choice.
    expect(html).not.toContain('data-testid="roll-clip"')
    expect(body(view('midi', { a: midiClip('A'), b: midiClip('B') }))).toContain('data-testid="roll-clip"')
  })
})

describe('the selection', () => {
  const a = { start: 0, pitch: 60, length: 480, velocity: 100 }
  const b = { ...a, pitch: 62 }

  it('toggles a note in and out', () => {
    const on = toggled(new Set(), noteKey(a))
    expect([...on.picked]).toEqual([noteKey(a)])
    expect(on.on).toBe(true)
    const off = toggled(on.picked, noteKey(a))
    expect(off.picked.size).toBe(0)
    expect(off.on).toBe(false)
  })

  it('follows the notes through a change: the old values leave, the new ones come in, the others stay', () => {
    const c = { ...a, pitch: 64 }
    const next = afterChanges(new Set([noteKey(a), noteKey(b)]), [{ from: a, to: c }])
    expect([...next].sort()).toEqual([noteKey(b), noteKey(c)].sort())
  })
})
