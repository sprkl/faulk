import { defineParams } from '@sveltejs/kit/params'
// With its extension: SvelteKit loads this file in Node during the build, outside Vite's resolver.
import { isLang } from './lib/i18n/lang.ts'

/**
 * Route parameter matchers:
 * - `[lang=lang]` accepts `en` and `fr` only;
 * - `[month=month]`, a Journey monthly archive: `2026-11`;
 * - `[day=day]`, a Journey entry's page: `<date>-<slug>`, `2026-11-02-neon-sur-la-rocade`.
 * Which months and days exist is decided by the routes' `entries()`, from the entry files.
 */
export const params = defineParams({
  lang: (param) => (isLang(param) ? param : undefined),
  month: (param) => (/^\d{4}-\d{2}$/.test(param) ? param : undefined),
  day: (param) => (/^\d{4}-\d{2}-\d{2}-[a-z0-9]+(?:-[a-z0-9]+)*$/.test(param) ? param : undefined),
})
