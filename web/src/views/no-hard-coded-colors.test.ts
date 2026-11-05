import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

/**
 * #687: the views and the components take every colour from the tokens (`lib/styles/tokens.css`), so that both
 * themes reach every pixel. This test reads each of their source files (tests aside), drops the comments, and lists
 * every colour written in it: a hex colour, a colour function, or a named colour as a CSS value or a string.
 */

const WEB = fileURLToPath(new URL('../..', import.meta.url))
const ROOTS = ['src/views', 'src/lib/components']

const blank = (text: string) => text.replace(/[^\n]/g, ' ')

/** The source without its comments (HTML, block and line ones), lines kept in place. */
export function stripComments(source: string): string {
  return (
    source
      .replace(/<!--[\s\S]*?-->/g, blank)
      .replace(/\/\*[\s\S]*?\*\//g, blank)
      // A line comment, not the `//` of a URL (`https://`) or one inside a string.
      .replace(/(^|[^:'"`\\])\/\/.*$/gm, '$1')
  )
}

const HEX = /(?<![\w&])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![\w-])/
const FUNCTION = /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/
const NAMES = 'white|black|red|green|blue|yellow|orange|purple|pink|gray|grey|silver|navy|teal|maroon'
// A named colour as a CSS value (`color: white`, `1px solid black`), not a class or an identifier.
const NAMED_VALUE = new RegExp(`:[^;{}\\n]*(?<![\\w.-])(?:${NAMES})(?![\\w-])[^;{}\\n]*;`)
// A named colour as a whole script string (`fill = 'white'`; the code is in single quotes, the markup's
// attributes, such as `class="white"`, in double ones).
const NAMED_STRING = new RegExp(`(['\`])(?:${NAMES})\\1`)

/** The lines of `source` that write a colour, numbered from 1. */
export function colorLiterals(source: string): { line: number; text: string }[] {
  const found: { line: number; text: string }[] = []
  stripComments(source)
    .split('\n')
    .forEach((text, i) => {
      if (HEX.test(text) || FUNCTION.test(text) || NAMED_VALUE.test(text) || NAMED_STRING.test(text)) found.push({ line: i + 1, text: text.trim() })
    })
  return found
}

function sources(): string[] {
  const files: string[] = []
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name)
      if (statSync(path).isDirectory()) walk(path)
      else if (/\.(svelte|ts)$/.test(name) && !/\.test\.ts$/.test(name)) files.push(path)
    }
  }
  for (const root of ROOTS) walk(join(WEB, root))
  return files
}

describe('no hard-coded colour in the views and the components (#687)', () => {
  it('reads every source file of both folders', () => {
    const files = sources().map((f) => relative(WEB, f).replace(/\\/g, '/'))
    // A sample of each folder and kind, so a broken walk cannot pass by reading nothing.
    for (const known of [
      'src/views/arrangement/ArrangementView.svelte',
      'src/views/arrangement/draw.ts',
      'src/views/piano-roll/Surface.svelte',
      'src/lib/components/LevelMeter.svelte',
      'src/lib/components/timegrid/render.ts',
    ])
      expect(files).toContain(known)
    expect(files.length).toBeGreaterThan(60)
  })

  it('finds none', () => {
    const found = sources().flatMap((file) =>
      colorLiterals(readFileSync(file, 'utf8')).map(({ line, text }) => `${relative(WEB, file).replace(/\\/g, '/')}:${line}: ${text}`),
    )
    expect(found).toEqual([])
  })

  it('catches each way of writing a colour, and nothing else', () => {
    for (const written of [
      'color: #fff;',
      'background: #1a1917;',
      "fill: '#ff8800cc'",
      'background: rgb(0 0 0 / 0.5);',
      'box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);',
      'color: hsl(20 10% 50%);',
      'color: oklch(0.74 0.13 20);',
      'color: white;',
      'border: 1px solid black;',
      "ctx.fillStyle = 'white'",
      'stroke="var(--fk-ink, #131211)"',
    ])
      expect(colorLiterals(written), written).toHaveLength(1)
    for (const fine of [
      '// the fix of #473 and #fff',
      '/* rgb(0 0 0) */',
      '<!-- #fff -->',
      'color: var(--fk-text);',
      'background: color-mix(in srgb, var(--track-color) 20%, var(--fk-bg));',
      '<div class="white"></div>',
      '.black {',
      'const black = isBlackKey(pitch)',
      'href="#main"',
      'white-space: nowrap;',
      "fetch('https://example.org/a') // #fff",
      'text-decoration: line-through;',
    ])
      expect(colorLiterals(fine), fine).toEqual([])
  })
})
