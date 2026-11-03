/**
 * The name of a new project (#473): drafts are no longer all "Untitled". A project started from the page is named
 * after the moment it was started, in the language of the page: "Sketch Nov 2, 2.05 PM", "Esquisse 2 nov., 14h05".
 * The time has no colon, which file names refuse (the `.faulkpack` and the audio export are named after the project).
 * The user renames it as any project; an agent may give `project.new` a name of its own.
 */
import type { Lang } from '../i18n/lang'

const WORD: Record<Lang, string> = { en: 'Sketch', fr: 'Esquisse' }

export function draftName(lang: Lang, at: Date): string {
  const day = new Intl.DateTimeFormat(lang, { month: 'short', day: 'numeric' }).format(at)
  const time = new Intl.DateTimeFormat(lang, { hour: 'numeric', minute: '2-digit' }).format(at).replace(':', lang === 'fr' ? 'h' : '.')
  return `${WORD[lang]} ${day}, ${time}`
}
