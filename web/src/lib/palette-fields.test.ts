import { describe, expect, it } from 'vitest'
import { buildCommand, fieldLabel } from './command-palette'
import { COMMAND_CATALOG, type CatalogEntry, type Note } from './generated/protocol'
import { messages } from './i18n/app'
import { translate, type MessageKey } from './i18n'
import { positionText, parseTickText, resolveFields, tickUnit, type Need, type PaletteContext } from './palette-fields'
import { HIDDEN_COMMANDS } from './palette-hidden'

const sig = { numerator: 4, denominator: 4 }
const BAR = 3840
const note = (pitch: number): Note => ({ start: 0, pitch, length: 960, velocity: 100, cents: 0, chance: 100, repeat: 1 }) as Note
const empty: PaletteContext = {
  selectedTrack: null,
  clip: null,
  scenes: [],
  markers: [],
  lanes: [],
  placements: [],
  notes: [],
  devices: [],
  signature: sig,
  current: {},
}
const full: PaletteContext = {
  selectedTrack: 'T1',
  clip: 'C1',
  placements: [{ id: 'P1', clip: 'C1' }],
  notes: [note(60), note(64)],
  devices: [{ id: 'D1', name: 'Oscillator' }],
  scenes: [{ id: 'S1', name: 'Scene 1' }],
  markers: [{ id: 'M1', name: 'Verse' }],
  lanes: [{ id: 'L1', name: 'Oscillator cutoff' }],
  signature: sig,
  current: { 'project.rename.name': 'My song', 'transport.set_tempo.bpm': '108' },
}
const entry = (name: string): CatalogEntry => COMMAND_CATALOG.find((c) => c.name === name)!
const visible = COMMAND_CATALOG.filter((c) => !HIDDEN_COMMANDS.has(c.name))
const NEEDS: Need[] = ['track', 'clip', 'placement', 'placements', 'notes', 'device', 'scene', 'lane', 'marker', 'preset']

describe('the forms of the palette fill themselves from the selection (#926)', () => {
  it('transposes the selected notes of the open clip without an identifier or JSON shown', () => {
    const { fields, missing } = resolveFields(entry('clip.notes.transpose'), full)
    expect(missing).toBeNull()
    expect(fields.filter((f) => f.shown).map((f) => f.field.name)).toEqual(['semitones'])
    const built = buildCommand(entry('clip.notes.transpose'), { ...Object.fromEntries(fields.map((f) => [f.field.name, f.text])), semitones: '2' })
    expect(built).toEqual({ ok: true, command: { type: 'clip.notes.transpose', clip: 'C1', notes: [note(60), note(64)], semitones: 2 } })
  })

  it('says what to select, shows no field and no JSON, when nothing usable is selected', () => {
    for (const [name, need] of [
      ['clip.notes.transpose', 'clip'],
      ['arrangement.delete', 'placements'],
      ['arrangement.split', 'placement'],
      ['device.set_bypass', 'device'],
      ['track.rename', 'track'],
      ['scene.rename', 'scene'],
      ['automation.lane.clear', 'lane'],
      ['arrangement.marker.delete', 'marker'],
    ] as const) {
      const { missing } = resolveFields(entry(name), empty)
      expect(missing, name).toBe(need)
    }
    expect(resolveFields(entry('clip.notes.transpose'), { ...empty, clip: 'C1' }).missing).toBe('notes')
  })

  it('takes the placement, or the several placements, selected in the timeline', () => {
    expect(resolveFields(entry('arrangement.split'), full).fields.find((f) => f.field.name === 'placement')?.text).toBe('P1')
    const two = {
      ...full,
      placements: [
        { id: 'P1', clip: 'C1' },
        { id: 'P2', clip: 'C2' },
      ],
    }
    expect(resolveFields(entry('arrangement.split'), two).missing).toBe('placement')
    expect(resolveFields(entry('arrangement.delete'), two).fields[0]!.text).toBe('["P1","P2"]')
  })

  it('asks which device when the track has several, never guesses, and fills the only one', () => {
    const many = {
      ...full,
      devices: [
        { id: 'D1', name: 'Oscillator' },
        { id: 'D2', name: 'Echo' },
      ],
    }
    const field = resolveFields(entry('device.remove'), many).fields[0]!
    expect(field.shown).toBe(true)
    expect(field.choices).toEqual([
      { value: 'D1', label: 'Oscillator' },
      { value: 'D2', label: 'Echo' },
    ])
    expect(resolveFields(entry('device.remove'), full).fields[0]).toMatchObject({ shown: false, text: 'D1' })
  })

  it('starts a field on its current value', () => {
    expect(resolveFields(entry('project.rename'), full).fields[0]!.text).toBe('My song')
    expect(resolveFields(entry('transport.set_tempo'), full).fields[0]!.text).toBe('108')
  })

  it('asks which scene, marker or lane, never guessing even when there is only one', () => {
    for (const [name, field] of [
      ['scene.delete', 'scene'],
      ['arrangement.marker.delete', 'marker'],
      ['automation.lane.clear', 'lane'],
    ] as const) {
      const item = resolveFields(entry(name), full).fields.find((f) => f.field.name === field)!
      expect(item.shown, name).toBe(true)
      expect(item.text, name).toBe('')
      expect(item.choices?.length, name).toBe(1)
      expect(buildCommand(entry(name), { [field]: '' }).ok, name).toBe(false)
    }
  })

  it('gives notes the open clip and the other commands the selected placement clip first', () => {
    const both = { ...full, clip: 'OPEN', placements: [{ id: 'P9', clip: 'PLACED' }] }
    expect(resolveFields(entry('clip.notes.transpose'), both).fields.find((f) => f.field.name === 'clip')?.text).toBe('OPEN')
    expect(resolveFields(entry('clip.rename'), both).fields.find((f) => f.field.name === 'clip')?.text).toBe('PLACED')
  })

  it('does not guess the destination track of a clip, but fills the track of a device', () => {
    expect(resolveFields(entry('clip.move'), full).fields.find((f) => f.field.name === 'track')?.text).toBe('')
    expect(resolveFields(entry('device.insert'), full).fields.find((f) => f.field.name === 'track')?.text).toBe('T1')
  })
})

describe('positions and lengths are typed in bars and beats (#926)', () => {
  it('reads positions like "Go to", lengths in bars and steps in beats', () => {
    expect(parseTickText('position', '33', sig, false)).toBe(32 * BAR)
    expect(parseTickText('position', '33.2', sig, false)).toBe(32 * BAR + 960)
    expect(parseTickText('position', '33.2.3', sig, false)).toBe(32 * BAR + 960 + 480)
    expect(parseTickText('position', '0', sig, false)).toBeNull()
    expect(parseTickText('position', 'soon', sig, false)).toBeNull()
    expect(parseTickText('bars', '2', sig, false)).toBe(2 * BAR)
    expect(parseTickText('bars', '0,5', sig, false)).toBe(BAR / 2)
    expect(parseTickText('bars', '-1', sig, true)).toBe(-BAR)
    expect(parseTickText('bars', '-1', sig, false)).toBeNull()
    expect(parseTickText('beats', '0.25', sig, false)).toBe(240)
    expect(positionText(32 * BAR + 960, sig)).toBe('33.2')
    expect(positionText(32 * BAR, sig)).toBe('33.1')
  })

  it('builds a command from such texts, for every tick field of the commands listed', () => {
    const seek = buildCommand(entry('transport.seek'), { tick: '33.2' }, sig)
    expect(seek).toEqual({ ok: true, command: { type: 'transport.seek', tick: 32 * BAR + 960 } })
    expect(buildCommand(entry('transport.seek'), { tick: 'soon' }, sig)).toEqual({ ok: false, problems: { tick: 'invalid' } })
    const waltz = { numerator: 3, denominator: 4 }
    expect(buildCommand(entry('transport.seek'), { tick: '2' }, waltz)).toEqual({ ok: true, command: { type: 'transport.seek', tick: 2880 } })
    const ticks = visible.flatMap((c) => c.fields.filter((f) => f.ref === 'Tick').map((f) => `${c.name}.${f.name}`))
    expect(ticks.length).toBeGreaterThan(10)
    for (const c of visible) for (const f of c.fields.filter((x) => x.ref === 'Tick')) expect(tickUnit(f), `${c.name}.${f.name}`).not.toBeNull()
  })
})

describe('what a musician is asked, for every command listed (#926)', () => {
  for (const lang of ['en', 'fr'] as const) {
    it(`labels every shown field in ${lang} and shows no identifier or JSON field, with or without a selection`, () => {
      const t = (key: MessageKey) => translate(lang, key)
      for (const c of visible)
        for (const ctx of [empty, full]) {
          for (const item of resolveFields(c, ctx).fields.filter((f) => f.shown)) {
            const f = item.field
            expect(f.kind, `${c.name}.${f.name}`).not.toBe('json')
            // No identifier or internal value is typed: such a field is a choice among named things.
            if (!item.choices) expect(f.ref ?? '', `${c.name}.${f.name}`).not.toMatch(/(Id|Hash|OrderKey|Ref)$/)
            const label = fieldLabel(t, c.name, f)
            expect(
              `palette.field.${c.name}.${f.name}` in messages[lang] || `palette.field.${f.name}` in messages[lang],
              `${lang} label of ${c.name}.${f.name}`,
            ).toBe(true)
            expect(label, c.name).not.toContain('_')
          }
        }
    })

    it(`has a sentence in ${lang} for everything a command can need selected`, () => {
      for (const need of NEEDS) expect(`palette.needs.${need}` in messages[lang], need).toBe(true)
      for (const source of ['track', 'clip', 'placement', 'placements', 'notes', 'device'])
        expect(`palette.uses.${source}` in messages[lang], source).toBe(true)
    })
  }

  it('lists no command with a required structured field the selection cannot fill', () => {
    for (const c of visible)
      for (const f of c.fields.filter((x) => x.kind === 'json' && x.required)) expect(['notes', 'placements'], `${c.name}.${f.name}`).toContain(f.name)
  })

  it('names the need of every identifier of a command listed', () => {
    for (const c of visible) {
      const { fields } = resolveFields(c, empty)
      for (const f of fields.filter((x) => x.missing)) expect(NEEDS, `${c.name}.${f.field.name}`).toContain(f.missing)
    }
  })
})
