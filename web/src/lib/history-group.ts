import type { Command } from './generated/protocol'
import { abortGroup, beginGroup, endGroup } from './track-commands'

/** How a caller runs a command: the dispatcher's `run` (or a host's), true when the command was accepted. */
export type GroupRun = (command: Command, options?: { quiet?: boolean }) => boolean

/**
 * Opens a history group and says whether this caller now owns it. A refused `history.begin_group` (a group is already
 * open, e.g. an agent's turn) is expected, not an error: it shows no refusal toast and the caller must not send
 * `history.end_group`, the open group is someone else's (#643, #653).
 */
export const openGroup = (run: GroupRun): boolean => run(beginGroup(), { quiet: true })

/** Closes the group `openGroup` returned `opened` for; does nothing when the group was not opened by this caller. */
export const closeGroup = (run: GroupRun, opened: boolean): void => {
  if (opened) run(endGroup())
}

/** What `runAll` did: every command applied, or `refused` is the index of the first refused (none after it was sent). */
export type RunAll = { ok: true } | { ok: false; refused: number; rolledBack: boolean }

/**
 * Runs `commands` in order as one history group, all or nothing (#867): the first refusal stops the batch and
 * `history.abort_group` takes back what was applied, leaving no history entry. The refusal itself shows its own message.
 * `rolledBack` is false only when the page could not undo (another author's group is open, or the abort was refused):
 * the group is then closed with `history.end_group` if the page owns it, and the earlier commands stay applied.
 */
export function runAll(run: GroupRun, commands: Command[]): RunAll {
  if (commands.length === 1) return run(commands[0]!) ? { ok: true } : { ok: false, refused: 0, rolledBack: true }
  const opened = openGroup(run)
  let refused = -1
  for (const [i, command] of commands.entries()) {
    if (!run(command)) {
      refused = i
      break
    }
  }
  if (refused < 0) {
    closeGroup(run, opened)
    return { ok: true }
  }
  // Only the group's author can abort it: with someone else's group open nothing can be taken back.
  const rolledBack = opened && run(abortGroup(), { quiet: true })
  if (opened && !rolledBack) closeGroup(run, true)
  return { ok: false, refused, rolledBack: rolledBack || refused === 0 }
}

/**
 * Runs `body` as one history group. When another group is open the body still runs (the history joins it to that group
 * only when the page itself opened it, otherwise each command is its own entry) and that group is left alone.
 */
export function inGroup<T>(run: GroupRun, body: () => T): T {
  const opened = openGroup(run)
  try {
    return body()
  } finally {
    closeGroup(run, opened)
  }
}
