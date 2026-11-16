import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { buildCommand, initialText, labelOf, MAX_RESULTS, PALETTE_LABEL_PREFIX, paletteEntries, searchCommands } from './command-palette'
import { COMMAND_CATALOG, type CatalogEntry } from './generated/protocol'
import { messages } from './i18n/app'
import { translate } from './i18n'

// Read at run time, never imported: schema.json is over a megabyte, and a JSON import makes the compiler infer its whole type (heap out of memory in the typecheck).
const schema = JSON.parse(readFileSync(new URL('./generated/schema.json', import.meta.url), 'utf8')) as {
  command: { catalog: { name: string; kind: string; input_schema: { required?: string[] } }[] }
}
const catalog = schema.command.catalog

const en = (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate('en', key, vars)
const entries = paletteEntries((key) => en(key))
const entry = (name: string): CatalogEntry => {
  const found = COMMAND_CATALOG.find((c) => c.name === name)
  if (!found) throw new Error(`no command ${name}`)
  return found
}

describe('the palette lists the whole catalogue', () => {
  it('has every edit, live and setting command of the Rust catalogue, and no query (the enumeration of #108)', () => {
    const wanted = catalog.filter((c) => c.kind !== 'query').map((c) => c.name)
    expect(wanted.length).toBeGreaterThan(100)
    expect(entries.map((e) => e.entry.name)).toEqual(wanted)
    for (const kind of ['edit', 'live']) expect(catalog.some((c) => c.kind === kind)).toBe(true)
  })

  it('finds every one of them by its own name, within the results it shows', () => {
    for (const { entry: e } of entries) {
      const names = searchCommands(entries, e.name)
        .slice(0, MAX_RESULTS)
        .map((r) => r.entry.name)
      expect(names, e.name).toContain(e.name)
    }
  })

  it('asks for exactly the required fields of each command', () => {
    for (const c of catalog.filter((c) => c.kind !== 'query')) {
      const required = (c.input_schema.required ?? []).slice().sort()
      expect(
        entry(c.name)
          .fields.filter((f) => f.required)
          .map((f) => f.name)
          .sort(),
        c.name,
      ).toEqual(required)
    }
  })

  it('gives every command a one-line summary', () => {
    for (const { entry: e } of entries) {
      expect(e.summary, e.name).not.toBe('')
      expect(e.summary, e.name).not.toContain('\n')
    }
  })
})

describe('labels', () => {
  it('only name commands the palette lists, in both languages', () => {
    const names = new Set(entries.map((e) => e.entry.name))
    for (const lang of ['en', 'fr'] as const) {
      const keys = Object.keys(messages[lang]).filter((k) => k.startsWith(PALETTE_LABEL_PREFIX))
      expect(keys.length).toBeGreaterThan(5)
      for (const key of keys) expect(names, key).toContain(key.slice(PALETTE_LABEL_PREFIX.length))
    }
  })

  it('prefers the palette label, then the history label, then the name in words', () => {
    expect(labelOf((k) => en(k), 'transport.play')).toBe('Play')
    expect(labelOf((k) => en(k), 'transport.set_tempo')).toBe(en('history.cmd.transport.set_tempo' as never))
    expect(labelOf((k) => en(k), 'live.all_notes_off')).toBe('live all notes off')
  })
})

describe('searchCommands', () => {
  const names = (query: string) => searchCommands(entries, query).map((e) => e.entry.name)

  it('lists everything in the catalogue order for an empty query', () => {
    expect(names('')).toEqual(entries.map((e) => e.entry.name))
    expect(names('   ')).toHaveLength(entries.length)
  })

  it('finds a command by a word of its name, in any order, ignoring case', () => {
    expect(names('TEMPO')).toContain('transport.set_tempo')
    expect(names('tempo set')).toContain('transport.set_tempo')
    expect(names('transport.set_tempo')[0]).toBe('transport.set_tempo')
  })

  it('ranks a word that starts a segment of the name before one found only in the summary', () => {
    const result = names('play')
    expect(result[0]).toBe('transport.play')
  })

  it('finds a command by its label, with or without accents', () => {
    expect(names('lecture')).toEqual([])
    const fr = paletteEntries((key) => translate('fr', key))
    const found = searchCommands(fr, 'lecture').map((e) => e.entry.name)
    expect(found[0]).toBe('transport.play')
    expect(searchCommands(fr, 'Arret').map((e) => e.entry.name)).toContain('transport.stop')
  })

  it('finds a command by a word of its summary', () => {
    expect(names('playhead')).toContain('transport.seek')
  })

  it('returns nothing for a word that is nowhere', () => {
    expect(names('zzzzqqqq')).toEqual([])
    expect(names('tempo zzzzqqqq')).toEqual([])
  })
})

describe('buildCommand', () => {
  it('makes the command of a name and a text field', () => {
    expect(buildCommand(entry('project.rename'), { name: 'My song' })).toEqual({ ok: true, command: { type: 'project.rename', name: 'My song' } })
  })

  it('reads whole numbers and numbers, and refuses what is not one', () => {
    const tempo = entry('transport.set_tempo')
    const field = tempo.fields.find((f) => f.required)!
    expect(field.kind).toBe('number')
    expect(buildCommand(tempo, { [field.name]: '128.5' })).toEqual({ ok: true, command: { type: 'transport.set_tempo', [field.name]: 128.5 } })
    expect(buildCommand(tempo, { [field.name]: 'fast' })).toEqual({ ok: false, problems: { [field.name]: 'invalid' } })
    expect(buildCommand(tempo, { [field.name]: 'Infinity' })).toEqual({ ok: false, problems: { [field.name]: 'invalid' } })
    const seek = entry('transport.seek')
    const tick = seek.fields[0]
    expect(tick.kind).toBe('integer')
    expect(buildCommand(seek, { [tick.name]: '960' }).ok).toBe(true)
    expect(buildCommand(seek, { [tick.name]: '1.5' })).toEqual({ ok: false, problems: { [tick.name]: 'invalid' } })
    expect(buildCommand(seek, { [tick.name]: '9007199254740993' })).toEqual({ ok: false, problems: { [tick.name]: 'invalid' } })
  })

  it('says a required field left empty needs a value, all of them at once', () => {
    const built = buildCommand(entry('project.rename'), {})
    expect(built).toEqual({ ok: false, problems: { name: 'required' } })
  })

  it('leaves out an optional field that is empty and keeps one that is given', () => {
    const optional = entry('project.new')
    expect(optional.fields).toEqual([expect.objectContaining({ name: 'name', required: false, kind: 'string' })])
    expect(buildCommand(optional, { name: '' })).toEqual({ ok: true, command: { type: 'project.new' } })
    expect(buildCommand(optional, { name: 'kept' })).toEqual({ ok: true, command: { type: 'project.new', name: 'kept' } })
  })

  it('takes a choice among the options only, and a switch as yes or no', () => {
    const theme = entry('view.set_theme')
    expect(theme.fields).toEqual([{ name: 'theme', required: true, ref: 'Theme', kind: 'enum', options: ['system', 'dark', 'light'] }])
    expect(buildCommand(theme, { theme: 'light' })).toEqual({ ok: true, command: { type: 'view.set_theme', theme: 'light' } })
    expect(buildCommand(theme, { theme: 'sepia' })).toEqual({ ok: false, problems: { theme: 'invalid' } })
    const monitor = entry('track.set_monitor')
    const flag = monitor.fields.find((f) => f.kind === 'boolean')!
    expect(buildCommand(monitor, { ...Object.fromEntries(monitor.fields.map((f) => [f.name, 'x'])), [flag.name]: 'true' })).toMatchObject({
      ok: true,
      command: { [flag.name]: true },
    })
    expect(buildCommand(monitor, { ...Object.fromEntries(monitor.fields.map((f) => [f.name, 'x'])), [flag.name]: 'maybe' })).toEqual({
      ok: false,
      problems: { [flag.name]: 'invalid' },
    })
  })

  it('reads a JSON field, and says when it is not JSON', () => {
    const json: CatalogEntry = { name: 'x.y', kind: 'edit', summary: '', fields: [{ name: 'list', required: true, kind: 'json' }] }
    expect(buildCommand(json, { list: '{ not json' })).toEqual({ ok: false, problems: { list: 'invalid' } })
    expect(buildCommand(json, { list: '[1, 2]' })).toEqual({ ok: true, command: { type: 'x.y', list: [1, 2] } })
    expect(buildCommand(json, { list: '{"a": {"b": null}}' })).toEqual({ ok: true, command: { type: 'x.y', list: { a: { b: null } } } })
    // Every json field of the catalogue is asked as JSON.
    expect(COMMAND_CATALOG.some((c) => c.fields.some((f) => f.kind === 'json'))).toBe(true)
  })

  it('reads null only when it is typed, never from an empty field (a level of null is silence)', () => {
    const select = entry('view.select_track')
    const field = select.fields[0]
    expect(field.nullable).toBe(true)
    expect(buildCommand(select, { [field.name]: '' })).toEqual({ ok: false, problems: { [field.name]: 'required' } })
    expect(buildCommand(select, { [field.name]: 'null' })).toEqual({ ok: true, command: { type: 'view.select_track', [field.name]: null } })
    // Every required nullable field of the catalogue answers the same, so no level silences by an empty Run.
    const nullable = COMMAND_CATALOG.filter((c) => c.fields.some((f) => f.required && f.nullable))
    expect(nullable.length).toBeGreaterThan(2)
    for (const c of nullable) {
      const f = c.fields.find((x) => x.required && x.nullable)!
      const others = Object.fromEntries(c.fields.filter((x) => x !== f).map((x) => [x.name, 'x']))
      expect(buildCommand(c, { ...others, [f.name]: '' }), c.name).toMatchObject({ ok: false, problems: { [f.name]: 'required' } })
    }
  })

  it('keeps the word null in a free text, and runs no flagged destructive command without asking for a field first', () => {
    expect(buildCommand(entry('project.new'), { name: 'null' })).toEqual({ ok: true, command: { type: 'project.new', name: 'null' } })
    // A command without fields runs at once, so none of them may be destructive (the warning is in the form).
    expect(COMMAND_CATALOG.filter((c) => c.destructive && c.fields.length === 0).map((c) => c.name)).toEqual([])
  })

  it('makes a command without fields as is', () => {
    expect(buildCommand(entry('transport.play'), {})).toEqual({ ok: true, command: { type: 'transport.play' } })
  })
})

describe('initialText', () => {
  it('fills a track identifier with the selected track, nothing else', () => {
    const field = COMMAND_CATALOG.flatMap((c) => c.fields).find((f) => f.ref === 'TrackId')!
    expect(initialText(field, { selectedTrack: '01ABC' })).toBe('01ABC')
    expect(initialText(field, { selectedTrack: null })).toBe('')
    const other = COMMAND_CATALOG.flatMap((c) => c.fields).find((f) => f.ref !== 'TrackId' && f.kind === 'integer')!
    expect(initialText(other, { selectedTrack: '01ABC' })).toBe('')
  })

  it('starts a required switch off and an optional one on its default', () => {
    const fields = COMMAND_CATALOG.flatMap((c) => c.fields).filter((f) => f.kind === 'boolean')
    for (const f of fields) expect(initialText(f, { selectedTrack: null })).toBe(f.required ? 'false' : '')
  })
})
