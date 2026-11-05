import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { trackInk } from '../components/timegrid/render'
import { TRACK_COLORS } from '../project/tracks'
import { CLIP_FILL_ALPHA } from '../../views/arrangement/draw'

/**
 * #687: the WCAG AA contrasts of the DAW's two palettes, computed from the stylesheets themselves (`tokens.css`, and
 * `app.css` for what the DAW sets apart). Text and the letters of controls: 4.5:1. Graphics a user reads (meters,
 * glyphs, the focus ring, a track's lines and notes): 3:1.
 */

const read = (name: string) => readFileSync(new URL(`./${name}`, import.meta.url), 'utf8')

/** The custom properties declared in the rule whose selector list is exactly `selector`. */
function block(css: string, selector: string): Record<string, string> {
  const flat = css.replace(/\/\*[\s\S]*?\*\//g, '')
  const out: Record<string, string> = {}
  for (const match of flat.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    if (match[1]!.replace(/\s+/g, ' ').trim() !== selector) continue
    for (const decl of match[2]!.matchAll(/(--fk-[\w-]+)\s*:\s*([^;]+);/g)) out[decl[1]!] = decl[2]!.trim()
  }
  return out
}

const tokens = read('tokens.css')
const base = block(tokens, ":root, [data-theme='light']")
const THEMES = {
  light: { ...base, ...block(read('app.css'), ":root[data-theme='light']") },
  dark: { ...base, ...block(tokens, "[data-theme='dark']") },
} as const

function rgb(hex: string): [number, number, number] {
  const h = hex.replace('#', '')
  const full = h.length === 3 ? [...h].map((c) => c + c).join('') : h
  expect(full, hex).toMatch(/^[0-9a-f]{6}$/i)
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16)) as [number, number, number]
}
function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((c) => {
    const v = c / 255
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!
}
export function ratio(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p)
  return (x! + 0.05) / (y! + 0.05)
}
/** `top` at `alpha` over `under`, as the canvas composes it. */
function over(top: string, under: string, alpha: number): string {
  const a = rgb(top)
  const b = rgb(under)
  return (
    '#' +
    a
      .map((v, i) =>
        Math.round(v * alpha + b[i]! * (1 - alpha))
          .toString(16)
          .padStart(2, '0'),
      )
      .join('')
  )
}

describe.each(Object.entries(THEMES))('the %s palette (#687)', (_, t) => {
  const v = (name: string) => {
    const value = t[name as keyof typeof t] as string | undefined
    expect(value, name).toBeDefined()
    return value!
  }
  const surfaces = ['--fk-bg', '--fk-surface', '--fk-surface-2']

  it('sets text, secondary text and accent text at 4.5:1 on every surface', () => {
    for (const fg of ['--fk-text', '--fk-text-muted', '--fk-accent-fg', '--fk-mute-fg', '--fk-rec-fg', '--fk-danger'])
      for (const bg of surfaces) expect(ratio(v(fg), v(bg)), `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5)
    expect(ratio(v('--fk-text-muted-on-tint'), v('--fk-accent-tint'))).toBeGreaterThanOrEqual(4.5)
    expect(ratio(v('--fk-text'), v('--fk-accent-tint'))).toBeGreaterThanOrEqual(4.5)
  })

  it('sets the letters on the accent, the signals and the track hues at 4.5:1', () => {
    for (const fill of ['--fk-accent', '--fk-accent-strong']) expect(ratio(v('--fk-on-accent'), v(fill)), fill).toBeGreaterThanOrEqual(4.5)
    for (const fill of ['--fk-play', '--fk-mute', '--fk-solo']) expect(ratio(v('--fk-on-signal'), v(fill)), fill).toBeGreaterThanOrEqual(4.5)
    for (const hue of TRACK_COLORS) expect(ratio(v('--fk-on-signal'), hue), hue).toBeGreaterThanOrEqual(4.5)
  })

  // Solo is not among them: its pressed state is the ink S and its underline on the fill, and the top bar's
  // waiting dot (`--fk-solo`) has an ink ring; the light yellow itself is pale on paper by design.
  it('draws meters, the record glyph and the focus ring at 3:1 on the background and the surface', () => {
    for (const fg of ['--fk-play', '--fk-mute', '--fk-rec', '--fk-accent-fg'])
      for (const bg of ['--fk-bg', '--fk-surface']) expect(ratio(v(fg), v(bg)), `${fg} on ${bg}`).toBeGreaterThanOrEqual(3)
  })

  it('draws each of the 16 track hues as lines and notes at 3:1 on the background and on a clip', () => {
    const shade = parseFloat(v('--fk-track-shade')) / 100
    const colors = { text: v('--fk-text'), trackShade: shade, bg: v('--fk-bg') }
    for (const hue of TRACK_COLORS) {
      const ink = trackInk(hue, colors)
      // The palette needs no more ink than the theme's shade: the floor of #698 (given the background) leaves it as it is.
      expect(trackInk(hue, { ...colors, bg: '' }), hue).toBe(ink)
      const clip = over(hue, v('--fk-bg'), CLIP_FILL_ALPHA)
      expect(ratio(ink, v('--fk-bg')), `${hue} on the background`).toBeGreaterThanOrEqual(3)
      expect(ratio(ink, clip), `${hue} on its clip`).toBeGreaterThanOrEqual(3)
    }
  })
})

describe('the two palettes', () => {
  it('keep the hues as they are in the dark theme and shade them only in the light one', () => {
    expect(THEMES.dark['--fk-track-shade']).toBe('0%')
    expect(trackInk(TRACK_COLORS[0], { text: THEMES.dark['--fk-text']!, trackShade: 0, bg: '' })).toBe(TRACK_COLORS[0])
    expect(parseFloat(THEMES.light['--fk-track-shade']!)).toBeGreaterThan(0)
  })
})
