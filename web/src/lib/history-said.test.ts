import { describe, expect, it } from 'vitest'
import type { HistoryEntryView } from './generated/protocol'
import { translate, type MessageKey } from './i18n'
import { commandLabel, runHistory } from './history-said'

const entry = (id: number, name: string): HistoryEntryView => ({ id, name, at: 0, author: 'user' })

/** A history of two stacks; `applies` says whether the next undo or redo does something (a group still open: it does not). */
function host(undo: HistoryEntryView[], redo: HistoryEntryView[], applies = true) {
  return {
    run(command: { type: string }) {
      if (!applies) return
      const [from, to] = command.type === 'history.undo' ? [undo, redo] : [redo, undo]
      const moved = from.pop()
      if (moved) to.push(moved)
    },
    history: () => ({ undo: [...undo], redo: [...redo] }),
  }
}

function announcer(lang: 'en' | 'fr' = 'en') {
  const said: string[] = []
  return {
    said,
    t: (key: MessageKey, vars?: Record<string, string | number>) => translate(lang, key, vars),
    announceHistory: (text: string) => said.push(text),
  }
}

describe('undo and redo in words (#494)', () => {
  it('says what an undo and a redo did, with the entry label', () => {
    const h = host([entry(1, 'clip.create'), entry(2, 'clip.notes.update')], [])
    const ui = announcer()
    runHistory(h, ui, 'undo')
    runHistory(h, ui, 'redo')
    expect(ui.said).toEqual(['Undone: Edit notes', 'Redone: Edit notes'])
  })

  it('says it in French', () => {
    const ui = announcer('fr')
    runHistory(host([entry(1, 'clip.notes.add')], []), ui, 'undo')
    expect(ui.said).toEqual(['Annulé : Ajouter des notes'])
  })

  it('says nothing when there is nothing to undo or redo, or when the history did not move', () => {
    const ui = announcer()
    runHistory(host([], []), ui, 'undo')
    runHistory(host([], []), ui, 'redo')
    runHistory(host([entry(1, 'clip.create')], [], false), ui, 'undo')
    expect(ui.said).toEqual([])
  })

  it('names a command without a label by its name in words', () => {
    expect(commandLabel((key) => translate('en', key), 'some.thing_new')).toBe('some thing new')
  })
})
