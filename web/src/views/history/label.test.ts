import { describe, expect, it } from 'vitest'
import { translate } from '../../lib/i18n/app'
import type { Subject } from '../../lib/generated/protocol'
import { label, nounOf } from './label'

const en = (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate('en', key, vars)
const fr = (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate('fr', key, vars)
const subject = (s: Partial<Subject>): Subject => ({ count: 1, objects: [], within: [], on: null, to: null, ...s })

describe('history labels that name the objects (#473)', () => {
  it('names a switched track', () => {
    expect(label({ name: 'mixer.set_mute', subject: subject({ objects: ['Lead'], on: true }) }, en)).toBe('Mute Lead')
    expect(label({ name: 'mixer.set_mute', subject: subject({ objects: ['Lead'], on: false }) }, en)).toBe('Unmute Lead')
    expect(label({ name: 'mixer.set_mute', subject: subject({ objects: ['Lead'], on: true }) }, fr)).toBe('Couper Lead')
    expect(label({ name: 'mixer.set_solo', subject: subject({ objects: ['Bass'], on: true }) }, fr)).toBe('Mettre Bass en solo')
  })

  it('counts many objects and names their tracks', () => {
    const many = subject({ count: 17, objects: ['Bass 1', 'Drums 1', 'Lead 1'], within: ['Bass', 'Drums', 'Lead'] })
    expect(label({ name: 'arrangement.delete', subject: many }, en)).toBe('Delete 17 clips (Bass, Drums, Lead)')
    expect(label({ name: 'arrangement.delete', subject: many }, fr)).toBe('Supprimer 17 clips (Bass, Drums, Lead)')
    const more = subject({ count: 5, within: ['A', 'B', 'C', 'D', 'E'] })
    expect(label({ name: 'arrangement.move', subject: more }, en)).toBe('Move 5 clips (A, B, C +2)')
  })

  it('names one clip with its track, and nameless objects by their noun', () => {
    expect(label({ name: 'clip.delete', subject: subject({ objects: ['Bass 1'], within: ['Bass'] }) }, en)).toBe('Delete Bass 1 (Bass)')
    expect(label({ name: 'clip.notes.add', subject: subject({ count: 4, within: ['Bass 1'] }) }, en)).toBe('Add 4 notes (Bass 1)')
    expect(label({ name: 'clip.notes.add', subject: subject({ count: 1, within: ['Bass 1'] }) }, fr)).toBe('Ajouter une note (Bass 1)')
  })

  it('says the new name or value', () => {
    expect(label({ name: 'track.rename', subject: subject({ objects: ['Synth'], to: 'Keys' }) }, en)).toBe('Rename Synth to Keys')
    expect(label({ name: 'transport.set_tempo', subject: subject({ count: 0, to: '128' }) }, en)).toBe('Set the tempo to 128 BPM')
    expect(label({ name: 'transport.toggle_loop', subject: subject({ count: 0, on: true }) }, en)).toBe('Turn the loop on')
  })

  it('falls back to the generic label, and counts the other edits of a group', () => {
    expect(label({ name: 'track.rename', subject: subject({ objects: ['Synth'] }) }, en)).toBe('Rename a track')
    expect(label({ name: 'mixer.set_mute', subject: null }, en)).toBe('Mute or unmute a track')
    expect(label({ name: 'arrangement.insert_time', subject: null }, en)).toBe('arrangement insert time')
    expect(label({ name: 'clip.notes.add', subject: subject({ count: 4, within: ['Bass 1'] }), others: 12 }, en)).toBe('Add 4 notes (Bass 1), +12 more')
    expect(label({ name: 'transport.set_metronome', subject: subject({ count: 0 }) }, en)).toBe('Change the metronome')
  })

  it('takes the noun from the command', () => {
    expect(nounOf('clip.notes.update')).toBe('note')
    expect(nounOf('automation.points.add')).toBe('point')
    expect(nounOf('automation.lane.clear')).toBe('lane')
    expect(nounOf('arrangement.place_scene')).toBe('scene')
    expect(nounOf('arrangement.delete')).toBe('clip')
    expect(nounOf('device.remove')).toBe('device')
    expect(nounOf('mixer.set_mute')).toBe('track')
  })
})
