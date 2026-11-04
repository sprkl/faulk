import type { Command } from './generated/protocol'
import { beginGroup, endGroup } from './track-commands'

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
