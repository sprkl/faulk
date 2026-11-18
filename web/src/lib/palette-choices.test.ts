import { describe, expect, it } from 'vitest'
import { buildCommand, fieldLabel } from './command-palette'
import { COMMAND_CATALOG, DEVICES, SCALE_INTERVALS, type CatalogEntry } from './generated/protocol'
import { translate, type MessageKey } from './i18n'
import { messages } from './i18n/app'
import { closedChoices, END_OF_LIST, hasList, isListIndex, NONE, OTHER } from './palette-choices'
import { resolveFields, usesKey, type PaletteContext } from './palette-fields'
import { HIDDEN_COMMANDS } from './palette-hidden'
import { TRACK_COLORS } from './project/tracks'

const sig = { numerator: 4, denominator: 4 }
const entry = (name: string): CatalogEntry => COMMAND_CATALOG.find((c) => c.name === name)!
const field = (command: string, name: string) => entry(command).fields.find((f) => f.name === name)!
const visible = COMMAND_CATALOG.filter((c) => !HIDDEN_COMMANDS.has(c.name))
const LANGS = ['en', 'fr'] as const
const tOf = (lang: (typeof LANGS)[number]) => (key: MessageKey, vars?: Record<string, string | number>) => translate(lang, key, vars)

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
  ...empty,
  selectedTrack: 'T1',
  clip: 'C1',
  devices: [{ id: 'D1', name: 'Oscillator' }],
  scenes: [{ id: 'S1', name: 'Scene 1' }],
  markers: [{ id: 'M1', name: 'Verse' }],
  lanes: [{ id: 'L1', name: 'Cutoff' }],
  placements: [{ id: 'P1', clip: 'C1' }],
}

describe('the closed choices of the palette come from the interface, in its language (#935)', () => {
  for (const lang of LANGS) {
    const t = tOf(lang)
    it(`offers the scales and tonics of the transport's Scale button in ${lang}, same order`, () => {
      const modes = closedChoices(entry('project.set_scale'), field('project.set_scale', 'mode'), t)!
      expect(modes.choices.map((c) => c.value)).toEqual(field('project.set_scale', 'mode').options)
      expect(modes.choices[0]).toEqual({ value: 'chromatic', label: t('scale.none') })
      expect(modes.choices.slice(1).map((c) => c.value)).toEqual(SCALE_INTERVALS.map((s) => s.mode))
      expect(modes.choices.find((c) => c.value === 'dorian')!.label).toBe(t('scale.mode.dorian'))
      const roots = closedChoices(entry('project.set_scale'), field('project.set_scale', 'root'), t)!
      expect(roots.choices).toHaveLength(12)
      expect(roots.choices[2]).toEqual({ value: '2', label: lang === 'fr' ? 'Ré' : 'D' })
    })

    it(`offers the sixteen named colours, "no colour" for a track that has one, and Other for a hexadecimal, in ${lang}`, () => {
      const set = closedChoices(entry('track.set_color'), field('track.set_color', 'color'), t)!
      expect(set.other).toBe(true)
      expect(set.choices[0]).toEqual({ value: NONE, label: t('palette.noColor') })
      expect(set.choices.slice(1).map((c) => c.value)).toEqual([...TRACK_COLORS])
      expect(set.choices.slice(1).every((c) => /^[A-Za-zÀ-ÿ ]+$/.test(c.label))).toBe(true)
      const made = closedChoices(entry('track.create'), field('track.create', 'color'), t)!
      expect(made.choices.map((c) => c.value)).toEqual([...TRACK_COLORS])
    })

    it(`offers the devices of the protocol's catalogue, by their one English name, in ${lang}`, () => {
      const instruments = closedChoices(entry('track.create'), field('track.create', 'instrument'), t)!
      expect(instruments.choices.map((c) => c.value)).toEqual(DEVICES.filter((d) => d.role === 'instrument').map((d) => d.kind))
      expect(instruments.choices.map((c) => c.label)).toEqual(DEVICES.filter((d) => d.role === 'instrument').map((d) => d.name))
      expect(closedChoices(entry('track.set_instrument'), field('track.set_instrument', 'instrument'), t)!.choices).toEqual(instruments.choices)
      const effects = closedChoices(entry('device.insert'), field('device.insert', 'kind'), t)!
      const names = effects.choices.map((c) => c.label)
      for (const d of DEVICES.filter((x) => (x.role as string) !== 'instrument')) expect(names, d.kind).toContain(d.name)
      expect(names.length).toBe(DEVICES.filter((x) => (x.role as string) !== 'instrument').length)
    })

    it(`names every option of every enum a command listed has, in ${lang}`, () => {
      for (const c of visible)
        for (const f of c.fields.filter((x) => x.kind === 'enum')) {
          const list = closedChoices(c, f, t)!
          for (const o of list.choices) {
            expect(o.label, `${c.name}.${f.name}.${o.value}`).not.toBe('')
            expect(o.label, `${c.name}.${f.name}.${o.value}`).not.toContain('_')
            if (f.ref !== 'ScaleKind') expect(`palette.opt.${f.ref}.${o.value}` in messages[lang], `${c.name}.${f.name}.${o.value}`).toBe(true)
          }
        }
    })
  }

  it('lists the knobs of the device the form is about, named as its panel names them', () => {
    const echo = DEVICES.find((d) => d.kind === 'faulk.echo')!
    const list = closedChoices(entry('device.set_param'), field('device.set_param', 'param'), tOf('fr'), { lang: 'fr', deviceKind: echo.kind })!
    expect(list.choices.map((c) => c.value)).toEqual(echo.params.map((p) => p.key))
    expect(list.choices.every((c) => c.label !== '' && !c.label.includes('_'))).toBe(true)
    expect(closedChoices(entry('device.set_param'), field('device.set_param', 'param'), tOf('en'))!.choices).toEqual([])
  })

  it('reads what the lists answer as the command the dispatcher runs', () => {
    expect(buildCommand(entry('project.set_scale'), { root: '9', mode: 'major' })).toEqual({
      ok: true,
      command: { type: 'project.set_scale', root: 9, mode: 'major' },
    })
    expect(buildCommand(entry('track.set_color'), { track: 'T1', color: TRACK_COLORS[1]! })).toEqual({
      ok: true,
      command: { type: 'track.set_color', track: 'T1', color: TRACK_COLORS[1] },
    })
    expect(buildCommand(entry('track.set_color'), { track: 'T1', color: NONE })).toEqual({
      ok: true,
      command: { type: 'track.set_color', track: 'T1', color: null },
    })
    expect(buildCommand(entry('track.set_color'), { track: 'T1', color: '#12ab34' })).toEqual({
      ok: true,
      command: { type: 'track.set_color', track: 'T1', color: '#12ab34' },
    })
    // A colour is never the word "none": a field left empty is an error, an optional one is absent.
    expect(buildCommand(entry('track.set_color'), { track: 'T1', color: '' }).ok).toBe(false)
    expect(buildCommand(entry('track.create'), { kind: 'midi' })).toEqual({ ok: true, command: { type: 'track.create', kind: 'midi' } })
  })

  it('shows "Other" only for the colours', () => {
    const t = tOf('en')
    const others = visible.flatMap((c) => c.fields.filter((f) => closedChoices(c, f, t)?.other).map((f) => `${c.name}.${f.name}`))
    expect(others.sort()).toEqual(['track.create.color', 'track.set_color.color'])
    expect(OTHER).not.toBe(NONE)
  })
})

describe('an optional field is left empty, a place in a list counts from 1 (#935)', () => {
  it('builds no value for an optional field left empty, and no "null" needs typing', () => {
    const built = buildCommand(entry('track.create'), { kind: 'audio', name: '', color: '', instrument: '' })
    expect(built).toEqual({ ok: true, command: { type: 'track.create', kind: 'audio' } })
    for (const lang of LANGS) expect(messages[lang]['palette.nullable' as MessageKey]).toBeDefined()
  })

  it('counts the place from 1, and puts it at the end when left empty', () => {
    expect(buildCommand(entry('device.insert'), { track: 'T1', kind: 'faulk.echo', index: '1' })).toMatchObject({ ok: true, command: { index: 0 } })
    expect(buildCommand(entry('device.insert'), { track: 'T1', kind: 'faulk.echo', index: '3' })).toMatchObject({ ok: true, command: { index: 2 } })
    expect(buildCommand(entry('device.insert'), { track: 'T1', kind: 'faulk.echo', index: '' })).toMatchObject({ ok: true, command: { index: END_OF_LIST } })
    for (const bad of ['0', '-1', '1.5', 'first'])
      expect(buildCommand(entry('device.insert'), { track: 'T1', kind: 'faulk.echo', index: bad }), bad).toEqual({ ok: false, problems: { index: 'invalid' } })
  })

  it('applies to every place in a list a listed command asks', () => {
    const places = visible.flatMap((c) => c.fields.filter(isListIndex).map((f) => `${c.name}.${f.name}`))
    expect(places.sort()).toEqual(['device.insert.index', 'device.move.index', 'scene.move.index', 'track.move.index'])
  })
})

describe('the sentence of a filled field agrees in number (#935)', () => {
  it('says one note or clip in the singular, several in the plural, in both languages', () => {
    expect(usesKey('notes', 1)).toBe('palette.uses.notes.one')
    expect(usesKey('notes', 2)).toBe('palette.uses.notes')
    expect(usesKey('placements', 1)).toBe('palette.uses.placements.one')
    expect(usesKey('track', 1)).toBe('palette.uses.track')
    expect(translate('fr', usesKey('notes', 1), { count: 1 })).toBe('Sur la note sélectionnée.')
    expect(translate('fr', usesKey('notes', 3), { count: 3 })).toBe('Sur les 3 notes sélectionnées.')
    expect(translate('en', usesKey('notes', 1), { count: 1 })).toBe('On the selected note.')
    for (const lang of LANGS) for (const key of ['palette.uses.notes.one', 'palette.uses.placements.one']) expect(key in messages[lang], key).toBe(true)
  })
})

describe('the palette speaks like the interface (#935)', () => {
  it('says device and master, never "appareil" or "sortie générale", in the palette texts', () => {
    for (const [key, text] of Object.entries(messages.fr))
      if (key.startsWith('palette.')) expect(text, key).not.toMatch(/appareil|sortie générale|panoramique général/i)
    expect(translate('fr', 'palette.cmd.device.insert')).toBe('Ajouter un device')
    expect(fieldLabel(tOf('fr'), 'device.insert', field('device.insert', 'master'))).toBe('Sur le master')
  })
})

/**
 * The fields a musician types for real: a name, a number, a web address, a position. Every other field a listed command shows
 * is a closed choice and must be a list. A new command whose field is neither fails here and says what to decide.
 */
const FREE_TEXT: Readonly<Record<string, string>> = {
  name: 'a name the musician invents',
  category: 'a name of a folder of presets',
  url: 'a web address',
  bpm: 'a number',
}

describe('no closed choice is left as free text (#935)', () => {
  it('lists, for every command listed, a shown field as a list unless it is a name, a number, a position or a web address', () => {
    const t = tOf('en')
    const text: string[] = []
    for (const c of visible)
      for (const ctx of [empty, full])
        for (const item of resolveFields(c, ctx).fields.filter((f) => f.shown)) {
          const f = item.field
          if (item.choices || closedChoices(c, f, t)) continue
          const where = `${c.name}.${f.name}`
          if (f.kind === 'integer' || f.kind === 'number') continue // a quantity: counts, levels, positions
          expect(f.kind, where).toBe('string')
          expect(FREE_TEXT, `${where} is typed as text: make it a list (palette-choices.ts) or say why it is free here`).toHaveProperty(f.name)
          text.push(where)
        }
    expect(text.length).toBeGreaterThan(5)
  })

  it('has no field the schema names a list for (kind, root, mode, colour, instrument, scale) that stays text', () => {
    const t = tOf('en')
    for (const c of visible)
      for (const f of c.fields) {
        if (f.kind === 'enum' || f.kind === 'boolean') expect(closedChoices(c, f, t), `${c.name}.${f.name}`).not.toBeNull()
        if (['color', 'instrument'].includes(f.name) || (f.name === 'kind' && f.kind === 'string') || c.name === 'project.set_scale')
          expect(hasList(c, f), `${c.name}.${f.name} is a closed choice typed as text`).toBe(true)
      }
  })

  it('keeps no list for a field that does not exist', () => {
    for (const key of [
      'project.set_scale.mode',
      'project.set_scale.root',
      'track.create.color',
      'track.set_color.color',
      'track.create.instrument',
      'track.set_instrument.instrument',
      'device.insert.kind',
    ]) {
      const [command, name] = [key.slice(0, key.lastIndexOf('.')), key.slice(key.lastIndexOf('.') + 1)]
      expect(field(command, name), key).toBeDefined()
    }
  })
})
