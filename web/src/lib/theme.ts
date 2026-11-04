/**
 * The theme of the DAW (#687): the user's choice (system, dark or light), kept by the browser for
 * every project, and the theme it resolves to, set as `data-theme` on the document where the tokens
 * (`styles/tokens.css`) read it. Dark is the default. In `system`, the theme follows the system's
 * `prefers-color-scheme`, also when it changes during the session. The store is `theme.svelte.ts`.
 */

import type { Command } from './generated/protocol'

/** localStorage key of the theme the user chose. */
export const THEME_KEY = 'faulk.theme'

export type ThemeChoice = 'system' | 'dark' | 'light'
export type ResolvedTheme = 'dark' | 'light'

export const THEME_CHOICES: readonly ThemeChoice[] = ['system', 'dark', 'light']

export const isThemeChoice = (value: unknown): value is ThemeChoice => value === 'system' || value === 'dark' || value === 'light'

/**
 * The theme a stored choice gives: light when it is `light`, or `system` while the system prefers light; else
 * dark (the default, also for a missing or unknown value). Must stay self-contained: its source is inlined in the
 * head of `/app` (`themeScript`), so the page is painted in its theme before the bundle runs.
 */
export function resolveTheme(stored: string | null, prefersLight: boolean): 'dark' | 'light' {
  if (stored === 'light') return 'light'
  if (stored === 'system' && prefersLight) return 'light'
  return 'dark'
}

/** The command of the project menu's theme items, and an agent's (`view.set_theme`). */
export const setTheme = (theme: ThemeChoice): Command => ({ type: 'view.set_theme', theme })
/** Whether a command is `view.set_theme`: the page answers it without a project (the dispatcher names no command). */
export const isSetTheme = (command: Command): command is Extract<Command, { type: 'view.set_theme' }> => command.type === 'view.set_theme'
/** What `view.set_theme` answers: its output, which carries nothing but its name. */
export const SET_THEME_OUTPUT = { type: 'view.set_theme' } as const

/** The media query `system` follows. */
export const LIGHT_QUERY = '(prefers-color-scheme: light)'

/**
 * The inline script of `/app`'s head (`hooks.server.ts`): sets `data-theme` from the stored choice before the
 * first paint. A browser that refuses storage or has no `matchMedia` gets the dark theme, never an error.
 */
export function themeScript(): string {
  return (
    `(function(){var s=null,l=false;try{s=localStorage.getItem(${JSON.stringify(THEME_KEY)})}catch(e){}` +
    `try{l=matchMedia(${JSON.stringify(LIGHT_QUERY)}).matches}catch(e){}` +
    `var t='dark';try{var pick=${resolveTheme.toString()};t=pick(s,l)}catch(e){}` +
    `document.documentElement.setAttribute('data-theme',t)})()`
  )
}
