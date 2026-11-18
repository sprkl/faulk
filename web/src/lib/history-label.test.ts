import { describe, expect, it } from 'vitest'
import { translate } from './i18n/app'
import type { Subject } from './generated/protocol'
import { label, nounOf } from './history-label'

const en = (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate('en', key, vars)
const fr = (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate('fr', key, vars)
const subject = (s: Partial<Subject>): Subject => ({
  count: 1,
  objects: [],
  within: [],
  unnamed: [],
  within_unnamed: [],
  on: null,
  to: null,
  param: null,
  value: null,
  ...s,
})

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

  it('names a take by its notes and tracks: one take, one entry', () => {
    expect(label({ name: 'record.commit_take', subject: subject({ count: 12, within: ['Bass', 'Drums'] }) }, en)).toBe('Record 12 notes (Bass, Drums)')
    expect(label({ name: 'record.commit_take', subject: subject({ count: 1, within: ['Keys'] }) }, fr)).toBe('Enregistrer une note (Keys)')
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

describe('history labels that name what was done, and on what (#934)', () => {
  it('says an unnamed track and scene by their default name, never as an empty name', () => {
    expect(label({ name: 'track.create', subject: subject({ unnamed: [2], within: ['Oscillator'] }) }, en)).toBe('Create Track 2 (Oscillator)')
    expect(label({ name: 'track.create', subject: subject({ unnamed: [2], within: ['Oscillator'] }) }, fr)).toBe('Créer Piste 2 (Oscillator)')
    expect(label({ name: 'track.delete', subject: subject({ unnamed: [3] }) }, fr)).toBe('Supprimer Piste 3')
    expect(label({ name: 'clip.create', subject: subject({ objects: ['Clip 1'], within_unnamed: [6] }) }, fr)).toBe('Créer Clip 1 (Piste 6)')
    expect(label({ name: 'scene.create', subject: subject({ unnamed: [2] }) }, en)).toBe('Create Scene 2')
    expect(label({ name: 'scene.delete', subject: subject({ unnamed: [1] }) }, fr)).toBe('Supprimer Scène 1')
  })

  it('names the lane an automation entry is about, parameter and track', () => {
    const cutoff = { kind: 'faulk.filter', device: 'Filter', key: 'cutoff' }
    const lane = (s: Partial<Subject> = {}) => subject({ param: cutoff, within: ['Lead'], ...s })
    expect(label({ name: 'automation.lane.create', subject: lane() }, en)).toBe('Create the lane Filter › Cutoff, Lead')
    expect(label({ name: 'automation.lane.delete', subject: lane() }, fr)).toBe('Supprimer le couloir Filter › Fréquence de coupure, Lead')
    expect(label({ name: 'automation.points.restore', subject: lane({ count: 8 }) }, en)).toBe('Write 8 automation points in Filter › Cutoff, Lead')
    expect(label({ name: 'automation.points.add', subject: lane({ count: 1 }) }, fr)).toBe(
      "Ajouter un point d'automation à Filter › Fréquence de coupure, Lead",
    )
    expect(label({ name: 'automation.points.remove', subject: lane({ count: 2, within: [], within_unnamed: [4] }) }, en)).toBe(
      'Remove 2 automation points from Filter › Cutoff, Track 4',
    )
    expect(label({ name: 'automation.lane.clear', subject: subject({ param: { kind: null, device: null, key: 'gain' }, within: ['Master'] }) }, fr)).toBe(
      'Vider le couloir Volume, Master',
    )
    // A lane that says no parameter keeps the generic label.
    expect(label({ name: 'automation.points.add', subject: subject({ within: ['Lead'] }) }, en)).toBe('Add automation points')
  })

  it('names the parameter and its new value', () => {
    const set = (key: string, value: number) =>
      subject({ objects: ['Filter'], within: ['Master'], param: { kind: 'faulk.filter', device: 'Filter', key }, value })
    expect(label({ name: 'device.set_param', subject: set('cutoff', 1200) }, en)).toBe('Set Cutoff of Filter (Master) to 1.20 kHz')
    expect(label({ name: 'device.set_param', subject: set('cutoff', 1200) }, fr)).toBe('Régler Fréquence de coupure de Filter (Master) à 1.20 kHz')
    // Without a value (a group of several changes), the generic phrase of the device.
    expect(label({ name: 'device.set_param', subject: subject({ objects: ['Filter'], within: ['Master'] }) }, en)).toBe('Change a parameter of Filter (Master)')
  })
})
