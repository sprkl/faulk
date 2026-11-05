import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { resolveTheme, setTheme, THEME_KEY, themeScript } from './theme'
import { browserThemeEnv, Theme } from './theme.svelte'
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

function env(stored: string | null, sys = system(false)) {
  const storage = new Set<(key: string | null) => void>()
  const e = {
    stored,
    storage,
    /** Another tab writes the store, then the browser tells this one. */
    elsewhere(value: string | null, key: string | null = THEME_KEY) {
      e.stored = value
      for (const f of [...storage]) f(key)
    },
    listen: (f: (key: string | null) => void) => {
      storage.add(f)
      return () => storage.delete(f)
    },
    read: () => e.stored,
    write: (value: string) => {
      e.stored = value
    },
    query: () => sys.query as unknown as MediaQueryList,
    root: () => document.documentElement,
  }
  return e
}

function contrastOf(a: string, b: string): number {
  const lum = (hex: string) => {
    const [r, g, bl] = [1, 3, 5].map((i) => {
      const v = parseInt(hex.slice(i, i + 2), 16) / 255
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
    })
    return 0.2126 * r! + 0.7152 * g! + 0.0722 * bl!
  }
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p)
  return (x! + 0.05) / (y! + 0.05)
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
    e.query = () => null as unknown as MediaQueryList
    expect(new Theme(e).resolved).toBe('dark')
  })

  it('follows a choice made in another tab, ignores other keys, and stops after dispose (#698)', () => {
    const e = env('dark')
    const page = new Theme(e)
    e.elsewhere('light')
    expect(page.choice).toBe('light')
    expect(shown()).toBe('light')
    // It reads the other tab's choice back; it does not write it again.
    expect(e.stored).toBe('light')
    e.elsewhere('dark', 'something.else')
    expect(shown()).toBe('light')
    e.elsewhere('dark')
    expect(shown()).toBe('dark')
    // A cleared store (key null) and a value it does not know give the default.
    e.elsewhere('light')
    e.elsewhere(null, null)
    expect(page.choice).toBe('dark')
    expect(shown()).toBe('dark')
    e.elsewhere('sepia')
    expect(page.choice).toBe('dark')
    page.dispose()
    expect(e.storage.size).toBe(0)
    e.elsewhere('light')
    expect(shown()).toBe('dark')
  })

  it("listens to the window's storage event by default (#698)", () => {
    const page = new Theme(browserThemeEnv)
    localStorage.setItem(THEME_KEY, 'light')
    window.dispatchEvent(new StorageEvent('storage', { key: THEME_KEY, newValue: 'light' }))
    expect(shown()).toBe('light')
    page.dispose()
    localStorage.setItem(THEME_KEY, 'dark')
    window.dispatchEvent(new StorageEvent('storage', { key: THEME_KEY, newValue: 'dark' }))
    expect(shown()).toBe('light')
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
  it('takes more ink for any colour until it holds 3:1 on the background and on the clip (#698)', () => {
    const colors = { text: '#131211', bg: '#fdfcfc', trackShade: 0.4 }
    // Without the background the fixed shade stays, as before.
    const plain = trackInk('#ffff00', { text: colors.text, trackShade: 0.4, bg: '' })
    const ink = trackInk('#ffff00', colors)
    expect(ink).not.toBe(plain)
    expect(contrastOf(ink, colors.bg)).toBeGreaterThanOrEqual(3)
    // The worst cases: white, and hues the palette never gives.
    for (const hue of ['#ffffff', '#ffff00', '#00ffff', '#fefefe', '#ffeeaa']) {
      const out = trackInk(hue, colors)
      expect(contrastOf(out, colors.bg), hue).toBeGreaterThanOrEqual(3)
      // On the clip's fill: the hue at 32 % over the background.
      const tint =
        '#' +
        [1, 3, 5]
          .map((i) =>
            Math.round(parseInt(hue.slice(i, i + 2), 16) * 0.32 + parseInt(colors.bg.slice(i, i + 2), 16) * 0.68)
              .toString(16)
              .padStart(2, '0'),
          )
          .join('')
      expect(contrastOf(out, tint), `${hue} on its clip`).toBeGreaterThanOrEqual(3)
    }
    // A colour that already reads keeps the plain shade.
    expect(trackInk('#000000', colors)).toBe(trackInk('#000000', { text: colors.text, trackShade: 0.4, bg: '' }))
  })

  it('is the hue itself in the dark theme and the hue with ink in the light one', () => {
    expect(trackInk('#f08098', { text: '#ece9e6', trackShade: 0, bg: '' })).toBe('#f08098')
    expect(trackInk('#ffffff', { text: '#000000', trackShade: 0.4, bg: '' })).toBe('#999999')
    // Asked again after a switch of theme, it is the new theme's.
    expect(trackInk('#ffffff', { text: '#000000', trackShade: 0.5, bg: '' })).toBe('#808080')
    expect(trackInk('#ffffff', { text: '#ffffff', trackShade: 0.5, bg: '' })).toBe('#ffffff')
  })

  it('mixes a computed colour too, as the waveform reads it from its style (#687)', () => {
    expect(trackInk('rgb(255, 255, 255)', { text: '#000000', trackShade: 0.4, bg: '' })).toBe('#999999')
    expect(trackInk('rgba(255, 255, 255, 0.5)', { text: 'rgb(0, 0, 0)', trackShade: 0.5, bg: '' })).toBe('#808080')
  })

  it('keeps a colour it cannot mix', () => {
    expect(trackInk('', { text: '#000000', trackShade: 0.4, bg: '' })).toBe('')
    expect(trackInk('#fff', { text: '#000000', trackShade: 0.4, bg: '' })).toBe('#fff')
  })
})
