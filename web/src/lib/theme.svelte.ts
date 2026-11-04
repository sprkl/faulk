/** The page's theme store (#687); the rule and the inline script of the head are in `theme.ts`. */

import { isThemeChoice, LIGHT_QUERY, resolveTheme, THEME_KEY, type ResolvedTheme, type ThemeChoice } from './theme'

/** The browser seams of the theme: the store and the system's preference. */
export interface ThemeEnv {
  read(): string | null
  write(value: string): void
  /** The system's preference query, `null` where `matchMedia` does not exist. */
  query(): Pick<MediaQueryList, 'matches' | 'addEventListener' | 'removeEventListener'> | null
  /** The element `data-theme` is set on, `null` outside a document. */
  root(): HTMLElement | null
}

export const browserThemeEnv: ThemeEnv = {
  read: () => {
    try {
      return localStorage.getItem(THEME_KEY)
    } catch {
      return null
    }
  },
  write: (value) => {
    try {
      localStorage.setItem(THEME_KEY, value)
    } catch {
      // Not kept: the theme still applies to this page.
    }
  },
  query: () => (typeof matchMedia === 'function' ? matchMedia(LIGHT_QUERY) : null),
  root: () => (typeof document === 'undefined' ? null : document.documentElement),
}

export class Theme {
  /** What the user chose. */
  choice = $state<ThemeChoice>('dark')
  /** What the page shows: the choice, or the system's preference in `system`. The canvases draw again when it changes. */
  resolved = $state<ResolvedTheme>('dark')
  private readonly media: ReturnType<ThemeEnv['query']>
  private readonly follow = () => this.apply()

  constructor(private readonly env: ThemeEnv = browserThemeEnv) {
    const stored = env.read()
    this.choice = isThemeChoice(stored) ? stored : 'dark'
    this.media = env.query()
    // One listener for the page's life: a change of the system's preference changes the theme only in `system`.
    this.media?.addEventListener('change', this.follow)
    this.apply()
  }

  /** Chooses the theme: kept by the browser, applied at once. */
  set(choice: ThemeChoice) {
    this.choice = choice
    this.env.write(choice)
    this.apply()
  }

  /** Resolves the choice and puts it on the document. */
  apply() {
    this.resolved = resolveTheme(this.choice, this.media?.matches ?? false)
    this.env.root()?.setAttribute('data-theme', this.resolved)
  }

  /** Stops following the system (tests). */
  dispose() {
    this.media?.removeEventListener('change', this.follow)
  }
}

let current: Theme | null = null

/** The page's theme, made on first use. */
export function theme(): Theme {
  current ??= new Theme()
  return current
}
