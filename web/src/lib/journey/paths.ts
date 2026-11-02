/**
 * The Journey's pages and their addresses. Every page exists three times: in English and French
 * under `/en` and `/fr`, and without a prefix (English, which moves a French reader to `/fr`), like
 * the landing page. Which months and days exist comes from the entry files (`entries.ts`), so the
 * routes' `entries()` prerender every one of them.
 */

import { resolve } from '$app/paths'
import type { Lang } from '../i18n/lang'
import { ENTRIES, folderOf, MONTHS } from './entries'

/** The main page, a monthly archive (`2026-11`), or one day's page (`2026-11-02-neon-sur-la-rocade`). */
export type JourneyView = { kind: 'home' } | { kind: 'month'; month: string } | { kind: 'day'; day: string }

/** A view's address in `lang`, or without a prefix (`undefined`). */
export function journeyHref(view: JourneyView, lang?: Lang): string {
  switch (view.kind) {
    case 'home':
      return lang ? resolve('/[lang=lang]/journey', { lang }) : resolve('/journey')
    case 'month':
      return lang ? resolve('/[lang=lang]/journey/[month=month]', { lang, month: view.month }) : resolve('/journey/[month=month]', { month: view.month })
    case 'day':
      return lang ? resolve('/[lang=lang]/journey/[day=day]', { lang, day: view.day }) : resolve('/journey/[day=day]', { day: view.day })
  }
}

/** Every page to prerender, without the language: the main page, each month, each day. */
export function journeyViews(): JourneyView[] {
  return [{ kind: 'home' }, ...MONTHS.map((month) => ({ kind: 'month' as const, month })), ...ENTRIES.map((e) => ({ kind: 'day' as const, day: folderOf(e) }))]
}
