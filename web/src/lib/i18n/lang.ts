/** Languages shared by the landing page and the DAW. */

export type Lang = 'en' | 'fr'
export const LANGS: readonly Lang[] = ['en', 'fr']

/** localStorage key of the language the user chose, shared by the landing page and the DAW. */
export const LANG_STORAGE_KEY = 'faulk.lang'

/**
 * The stored choice if valid, else French when the browser's language is French, else English.
 * Must stay self-contained: its source is inlined in the landing page's detection script.
 */
export function defaultLang(stored: string | null, browser: string | undefined): Lang {
  if (stored === 'en' || stored === 'fr') return stored
  return browser?.toLowerCase().startsWith('fr') ? 'fr' : 'en'
}

/** Replaces `{name}` placeholders; unknown names are left as they are. */
export function fill(text: string, vars: Record<string, string | number> = {}): string {
  return text.replace(/\{(\w+)\}/g, (_, name: string) => String(vars[name] ?? `{${name}}`))
}

export function isLang(value: string | undefined): value is Lang {
  return value === 'en' || value === 'fr'
}
