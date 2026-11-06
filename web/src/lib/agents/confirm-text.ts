import { MAX_AUDIO_URL_BYTES } from '../generated/protocol'
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
    case 'importAudio':
      // The host alone in the title: the file's name, the agent's own words, never sits next to it.
      return t('agentConfirm.importAudio.title', { agent, host: q.host })
  }
}

/** The page's cap on a file downloaded from a link, in MB as the texts say it (MiB). */
export const MAX_AUDIO_URL_MB = Math.round(MAX_AUDIO_URL_BYTES / (1 << 20))

const BODIES: Record<PendingQuestion['question']['kind'], MessageKey> = {
  new: 'agentConfirm.replace.body',
  open: 'agentConfirm.replace.body',
  delete: 'agentConfirm.delete.body',
  keepMine: 'agentConfirm.keepMine.body',
  loadFolder: 'agentConfirm.loadFolder.body',
  command: 'agentConfirm.command.body',
  importAudio: 'agentConfirm.importAudio.body',
}

/** What the user loses if they allow it. */
export function questionBody(pending: PendingQuestion, t: Translate): string {
  const q = pending.question
  if (q.kind === 'importAudio') {
    // As the track list shows it (`trackLabel`): its name, or its default name by its position.
    const track = q.track.name.trim() || t('tracks.defaultName', { n: q.track.index + 1 })
    return t(BODIES[q.kind], { name: q.name, host: q.host, track, max: MAX_AUDIO_URL_MB })
  }
  return t(BODIES[q.kind], { name: 'target' in q ? (q.target ?? '') : '' })
}
