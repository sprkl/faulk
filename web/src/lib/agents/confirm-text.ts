import type { MessageKey } from '../i18n/app'
import type { PendingQuestion } from './confirm.svelte'

type Translate = (key: MessageKey, vars?: Record<string, string | number>) => string

/** The agent's name, as its MCP client gave it (`agent:<name>`). */
export function agentName(author: string): string {
  return author.replace(/^agent:/, '')
}

/** The title of an agent's question (AG-4): who asks, and what it would do. */
export function questionTitle(pending: PendingQuestion, t: Translate): string {
  const agent = agentName(pending.agent)
  const q = pending.question
  switch (q.kind) {
    case 'new':
      return t('agentConfirm.new.title', { agent })
    case 'open':
      return q.target ? t('agentConfirm.open.titleNamed', { agent, name: q.target }) : t('agentConfirm.open.title', { agent })
    case 'delete':
      return q.target ? t('agentConfirm.delete.titleNamed', { agent, name: q.target }) : t('agentConfirm.delete.title', { agent })
    case 'keepMine':
      return t('agentConfirm.keepMine.title', { agent })
    case 'loadFolder':
      return t('agentConfirm.loadFolder.title', { agent })
    case 'command':
      return t('agentConfirm.command.title', { agent, command: q.command })
  }
}

const BODIES: Record<PendingQuestion['question']['kind'], MessageKey> = {
  new: 'agentConfirm.replace.body',
  open: 'agentConfirm.replace.body',
  delete: 'agentConfirm.delete.body',
  keepMine: 'agentConfirm.keepMine.body',
  loadFolder: 'agentConfirm.loadFolder.body',
  command: 'agentConfirm.command.body',
}

/** What the user loses if they allow it. */
export function questionBody(pending: PendingQuestion, t: Translate): string {
  const q = pending.question
  return t(BODIES[q.kind], { name: 'target' in q ? (q.target ?? '') : '' })
}
