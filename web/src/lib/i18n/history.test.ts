import { describe, expect, it } from 'vitest'
import schema from '../generated/schema.json'
import { messages } from './app'
import { COMMAND_LABEL_PREFIX } from './history'

const catalog = schema.command.catalog as { name: string; kind: string }[]
const labelled = (lang: 'en' | 'fr') =>
  Object.keys(messages[lang])
    .filter((key) => key.startsWith(COMMAND_LABEL_PREFIX))
    .map((key) => key.slice(COMMAND_LABEL_PREFIX.length))

describe('history labels', () => {
  it('only name commands that exist and are recorded in the history, in both languages', () => {
    // history.undo and history.redo are never recorded: no entry carries their name.
    const recorded = new Set(catalog.filter((c) => c.kind === 'edit' && !c.name.startsWith('history.')).map((c) => c.name))
    for (const lang of ['en', 'fr'] as const) for (const name of labelled(lang)) expect(recorded, `${lang}: ${name}`).toContain(name)
    expect(labelled('fr')).toEqual(labelled('en'))
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
