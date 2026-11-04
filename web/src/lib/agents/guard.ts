import { AGENT_CONFIRM_MS, DESTRUCTIVE_COMMANDS, type AgentResult, type Command } from '../generated/protocol'
import type { AgentQuestion, ConfirmAnswer } from './confirm.svelte'

/**
 * Which agent calls wait for the user's answer in the page (AG-4, #100), and what an answer other than "allow" tells
 * the agent. A user's own click is its own confirmation: these questions are asked for agents only.
 *
 * Two families:
 * - the `x-destructive` commands (`DESTRUCTIVE_COMMANDS`, from the catalog: `project.new`, `project.open`);
 * - the page's storage actions that replace the open project, delete a project kept in the browser or settle a conflict
 *   with the project's folder (`STORAGE_GUARDED`). They are `live` (the contract keeps the destructive flag for edits),
 *   but each replaces or loses work as `project.open` does.
 */

/** What the storage tells the questions: the names of the projects they are about. */
export interface GuardView {
  recent: readonly { id: string; name: string }[]
  browserProjects: readonly { dir: string; name: string }[]
  /** The project the reopen banner offers, or the one that last failed to reopen. */
  reopenTarget: { name: string } | null
  folderName: string | null
  /** Whether the project's folder is in conflict now (only then do `keep_mine` and `load_folder_version` act). */
  inConflict: boolean
}

/** The storage actions an agent's call of which waits for the user's answer, by the question they ask. */
export const STORAGE_GUARDED: Readonly<Record<string, AgentQuestion['kind']>> = {
  'project.new': 'new',
  'project.open_recent': 'open',
  'project.open_browser_project': 'open',
  'project.reopen_last': 'open',
  'project.open_url': 'open',
  'project.delete_browser_project': 'delete',
  'project.keep_mine': 'keepMine',
  'project.load_folder_version': 'loadFolder',
}

/** The host of a link, for the question; `null` for one that does not parse (the storage refuses it then). */
function hostOf(url: unknown): string | null {
  if (typeof url !== 'string') return null
  try {
    return new URL(url).host || null
  } catch {
    return null
  }
}

/**
 * The question an agent's `command` waits for, or `null` when it runs at once. A way out of a conflict while there
 * is none asks nothing: the storage refuses it without changing anything.
 */
export function agentQuestion(command: Command, view: GuardView | null): AgentQuestion | null {
  const type = command.type
  const fields = command as unknown as Record<string, unknown>
  const kind = STORAGE_GUARDED[type]
  if (kind === undefined) return DESTRUCTIVE_COMMANDS.includes(type) ? { kind: 'command', command: type } : null
  switch (kind) {
    case 'new':
      return { kind: 'new' }
    case 'delete':
      return { kind: 'delete', target: view?.browserProjects.find((p) => p.dir === fields.dir)?.name ?? null }
    case 'keepMine':
    case 'loadFolder':
      if (view && !view.inConflict) return null
      return { kind, target: view?.folderName ?? null }
    case 'open': {
      const target =
        type === 'project.open_recent'
          ? view?.recent.find((r) => r.id === fields.id)?.name
          : type === 'project.open_browser_project'
            ? view?.browserProjects.find((p) => p.dir === fields.dir)?.name
            : type === 'project.reopen_last'
              ? view?.reopenTarget?.name
              : hostOf(fields.url)
      return { kind: 'open', target: target ?? null }
    }
    case 'command':
      return { kind: 'command', command: type }
  }
}

/** The deadline of a call that reached the page at `now`: its question is answered `timeout` past it. */
export function confirmDeadline(now: number): number {
  return now + AGENT_CONFIRM_MS
}

/** What the agent is told when the user did not allow its call: a typed refusal, nothing changed. */
export function notConfirmed(answer: Exclude<ConfirmAnswer, 'confirmed'>): AgentResult {
  switch (answer) {
    case 'declined':
      return { status: 'refused', error: { code: 'confirmation.declined', message: 'the user refused it in the page: nothing changed' } }
    case 'timeout':
      return {
        status: 'refused',
        error: {
          code: 'confirmation.timeout',
          message: `nobody answered the question in the page within ${AGENT_CONFIRM_MS / 1000} s: nothing changed`,
        },
      }
    case 'withdrawn':
      // The link that carried the call closed: this answer reaches nobody, it only ends the call.
      return { status: 'unavailable', reason: 'the call ended before the user answered: nothing changed' }
  }
}
