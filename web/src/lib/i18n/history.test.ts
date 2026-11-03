import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { messages } from './app'
import { COMMAND_LABEL_PREFIX, PHRASE_PREFIX } from './history'

// Read at run time, never imported: schema.json is tens of MB, and a JSON import makes the compiler infer its whole type (heap out of memory in the typecheck).
const schema = JSON.parse(readFileSync(new URL('../generated/schema.json', import.meta.url), 'utf8')) as {
  command: { catalog: { name: string; kind: string }[] }
}
const catalog = schema.command.catalog
const labelled = (lang: 'en' | 'fr') =>
  Object.keys(messages[lang])
    .filter((key) => key.startsWith(COMMAND_LABEL_PREFIX))
    .map((key) => key.slice(COMMAND_LABEL_PREFIX.length))

describe('history labels', () => {
  it('only name commands that exist and are recorded in the history, in both languages', () => {
    // history.undo and history.redo are never recorded: no entry carries their name.
    // A setting (#473) is shown too, in the activity log.
    const recorded = new Set(catalog.filter((c) => (c.kind === 'edit' || c.kind === 'setting') && !c.name.startsWith('history.')).map((c) => c.name))
    for (const lang of ['en', 'fr'] as const) for (const name of labelled(lang)) expect(recorded, `${lang}: ${name}`).toContain(name)
    expect(labelled('fr')).toEqual(labelled('en'))
  })

  it('phrase only commands that are recorded, with the placeholders of their English text in French', () => {
    const recorded = new Set(catalog.filter((c) => c.kind === 'edit' || c.kind === 'setting').map((c) => c.name))
    const phrases = Object.keys(messages.en).filter((key) => key.startsWith(PHRASE_PREFIX))
    expect(phrases.length).toBeGreaterThan(40)
    for (const key of phrases) {
      const name = key.slice(PHRASE_PREFIX.length).replace(/\.(on|off)$/, '')
      expect(recorded, key).toContain(name)
      const holes = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort()
      expect(holes(messages.fr[key as keyof typeof messages.fr]), key).toEqual(holes(messages.en[key as keyof typeof messages.en]))
    }
  })

  it('labels the commands the editing views issue today', () => {
    for (const name of [
      'clip.notes.add',
      'clip.notes.update',
      'clip.notes.remove',
      'device.set_param',
      'mixer.set_volume',
      'track.create',
      'transport.set_tempo',
    ]) {
      expect(labelled('en')).toContain(name)
    }
  })
})
