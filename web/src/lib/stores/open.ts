import type { FolderError } from '../generated/protocol'
import type { MessageKey } from '../i18n'

/** Why a folder did not open: the folder's refusal, or `frame` when the page's entries are not a folder. */
export type OpenRefusal = FolderError['reason'] | 'frame'

/** What `Dispatcher.openProject` came to: the project that replaced the current one, or why it did not. */
export type OpenOutcome = { status: 'opened'; name: string } | { status: 'refused'; reason: OpenRefusal }

/** The notice that tells the user why a folder did not open. */
export function refusalNotice(reason: OpenRefusal): MessageKey {
  if (reason === 'newer_format') return 'project.open.newer'
  if (reason === 'not_a_project') return 'project.open.notAProject'
  if (reason === 'io' || reason === 'too_large') return 'project.open.unreadable'
  return 'project.open.invalid'
}

/** The notice for a project opened in Boost: the page plays it, in Web mode. */
export const WEB_ONLY_NOTICE: MessageKey = 'project.open.webOnly'
