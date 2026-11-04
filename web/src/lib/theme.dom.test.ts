import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { resolveTheme, setTheme, THEME_KEY, themeScript } from './theme'
import { browserThemeEnv, Theme, type ThemeEnv } from './theme.svelte'
import { trackInk } from './components/timegrid/render'

/** A system preference the test flips, with the listeners the page added. */
function system(light: boolean) {
  const listeners = new Set<() => void>()
  const query = {
    matches: light,
    addEventListener: (_: string, f: () => void) => listeners.add(f),
    removeEventListener: (_: string, f: () => void) => listeners.delete(f),
  }
  return {
    query,
    listeners,
    flip(next: boolean) {
      query.matches = next
      for (const f of [...listeners]) f()
    },
  }
}

function env(stored: string | null, sys = system(false)): ThemeEnv & { stored: string | null } {
  const e = {
    stored,
    read: () => e.stored,
    write: (value: string) => {
      e.stored = value
    },
    query: () => sys.query as unknown as MediaQueryList,
    root: () => document.documentElement,
  }
  return e
}

const shown = () => document.documentElement.getAttribute('data-theme')

describe('the theme of the DAW (#687)', () => {
  beforeEach(() => document.documentElement.setAttribute('data-theme', 'dark'))
  afterEach(() => localStorage.clear())

  it('is dark by default, and for a stored value it does not know', () => {
    expect(new Theme(env(null)).resolved).toBe('dark')
    const odd = new Theme(env('sepia', system(true)))
    expect(odd.choice).toBe('dark')
    expect(odd.resolved).toBe('dark')
    expect(shown()).toBe('dark')
  })

  it('applies each choice at once, keeps it, and comes back with it on the next page', () => {
    const e = env(null)
    const page = new Theme(e)
    page.set('light')
    expect(shown()).toBe('light')
    expect(e.stored).toBe('light')
    expect(new Theme(e).resolved).toBe('light')
    page.set('dark')
    expect(shown()).toBe('dark')
    expect(e.stored).toBe('dark')
  })

  it('in system, follows the system both ways during the session, and stops when another choice is made', () => {
    const sys = system(false)
    const page = new Theme(env('system', sys))
    expect(page.resolved).toBe('dark')
    sys.flip(true)
    expect(page.resolved).toBe('light')
    expect(shown()).toBe('light')
    sys.flip(false)
    expect(shown()).toBe('dark')
    page.set('dark')
    sys.flip(true)
    expect(page.resolved).toBe('dark')
    expect(shown()).toBe('dark')
    page.set('system')
    expect(shown()).toBe('light')
    page.dispose()
    expect(sys.listeners.size).toBe(0)
  })

  it('works where the browser refuses storage: dark, and a choice still applies to the page', () => {
    const get = Storage.prototype.getItem
    const set = Storage.prototype.setItem
    const refused = () => {
      throw new Error('refused')
    }
    Storage.prototype.getItem = refused
    Storage.prototype.setItem = refused
    try {
      const page = new Theme(browserThemeEnv)
      expect(page.resolved).toBe('dark')
      page.set('light')
      expect(shown()).toBe('light')
      page.dispose()
    } finally {
      Storage.prototype.getItem = get
      Storage.prototype.setItem = set
    }
  })

  it('works without matchMedia: system is dark', () => {
    const e = env('system')
    e.query = () => null
    expect(new Theme(e).resolved).toBe('dark')
  })

  it('is set by the command the menu and the agents send', () => {
    expect(setTheme('light')).toEqual({ type: 'view.set_theme', theme: 'light' })
  })
})

describe('the script of the head of /app (#687)', () => {
  const run = (stored: string | null, light: boolean | 'missing', refuse = false) => {
    document.documentElement.setAttribute('data-theme', 'dark')
    localStorage.clear()
    if (stored !== null) localStorage.setItem(THEME_KEY, stored)
    const g = globalThis as { matchMedia?: unknown }
    const saved = g.matchMedia
    const savedGet = Storage.prototype.getItem
    g.matchMedia = light === 'missing' ? undefined : () => ({ matches: light })
    if (refuse)
      Storage.prototype.getItem = () => {
        throw new Error('refused')
      }
    try {
      new Function(themeScript())()
    } finally {
      g.matchMedia = saved
      Storage.prototype.getItem = savedGet
    }
    return shown()
  }

  it('gives the theme the store gives, for every stored value and system preference', () => {
    for (const stored of [null, 'dark', 'light', 'system', 'sepia'])
      for (const light of [false, true]) expect(run(stored, light), `${stored} ${light}`).toBe(resolveTheme(stored, light))
  })

  it('never throws: refused storage or no matchMedia give dark', () => {
    expect(run('light', true, true)).toBe('dark')
    expect(run('system', 'missing')).toBe('dark')
    expect(run('light', 'missing')).toBe('light')
  })
})

describe('a track hue drawn as a line (#687)', () => {
  it('is the hue itself in the dark theme and the hue with ink in the light one', () => {
    expect(trackInk('#f08098', { text: '#ece9e6', trackShade: 0 })).toBe('#f08098')
    expect(trackInk('#ffffff', { text: '#000000', trackShade: 0.4 })).toBe('#999999')
    // Asked again after a switch of theme, it is the new theme's.
    expect(trackInk('#ffffff', { text: '#000000', trackShade: 0.5 })).toBe('#808080')
    expect(trackInk('#ffffff', { text: '#ffffff', trackShade: 0.5 })).toBe('#ffffff')
  })

  it('keeps a colour it cannot mix', () => {
    expect(trackInk('', { text: '#000000', trackShade: 0.4 })).toBe('')
    expect(trackInk('#fff', { text: '#000000', trackShade: 0.4 })).toBe('#fff')
  })
})
