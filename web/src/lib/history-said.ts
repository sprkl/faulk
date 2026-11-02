/**
 * Undo and redo in words (#494): what an undo or a redo did, for the polite live regions of the timeline and the piano roll
 * ("Undone: Add notes"). The label is the history entry's (its first command's, as the history panel shows it).
 */

import type { HistoryEntryView } from './generated/protocol'
import { messages } from './i18n/app'
import { COMMAND_LABEL_PREFIX } from './i18n/history'
import type { MessageKey } from './i18n'
import { redo, undo } from './track-commands'

/** The label of a history entry: its first command's, translated; a command without one shows its name in words. */
export function commandLabel(t: (key: MessageKey) => string, name: string): string {
  const key = COMMAND_LABEL_PREFIX + name
  if (key in messages.en) return t(key as MessageKey)
  return name.replace(/[._]/g, ' ')
}

/** What runs a history command and reads the history: the dispatcher. */
interface HistoryHost {
  run(command: ReturnType<typeof undo>, options?: { quiet?: boolean }): void
  history(): { undo: HistoryEntryView[]; redo: HistoryEntryView[] }
}

/** Where the words go: the UI store. */
interface Announcer {
  t: (key: MessageKey, vars?: Record<string, string | number>) => string
  announceHistory(text: string): void
}

/**
 * Runs an undo or a redo, then says what it did when it did something: the entry it took moved to the other stack. An undo
 * that did nothing (an empty history, a history group still open) says nothing.
 */
export function runHistory(host: HistoryHost, ui: Announcer, action: 'undo' | 'redo', options: { quiet?: boolean } = {}): void {
  const before = host.history()
  const entry = (action === 'undo' ? before.undo : before.redo).at(-1)
  host.run(action === 'undo' ? undo() : redo(), options)
  if (!entry) return
  const after = host.history()
  const moved = (action === 'undo' ? after.redo : after.undo).at(-1)
  if (moved?.id !== entry.id) return
  const what = commandLabel(ui.t, entry.name)
  ui.announceHistory(ui.t(action === 'undo' ? 'history.saidUndone' : 'history.saidRedone', { what }))
}
