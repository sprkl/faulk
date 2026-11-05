import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

/**
 * #698: a `var(--fk-…)` with no fallback names a token that is declared somewhere (the stylesheets, or a rule or a
 * `style:--fk-…` of the component that sets it). `--fk-radius` and `--fk-radius-sm` were used and never declared,
 * which gave square corners.
 */

const SRC = fileURLToPath(new URL('../..', import.meta.url))

function files(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) {
      if (name !== 'generated' && name !== 'node_modules') files(path, out)
    } else if (/\.(svelte|css|ts)$/.test(name) && !/\.test\.ts$/.test(name)) out.push(path)
  }
  return out
}

describe('the design tokens (#698)', () => {
  const sources = files(SRC).map((path) => ({ path: relative(SRC, path).replace(/\\/g, '/'), text: readFileSync(path, 'utf8') }))
  const declared = new Set<string>()
  // A declaration is in a stylesheet or a component's style, not in a comment: block and HTML comments are dropped.
  const bare = (text: string) => text.replace(/<!--[\s\S]*?-->/g, '').replace(/\/\*[\s\S]*?\*\//g, '')
  for (const { path, text } of sources)
    if (/\.(svelte|css)$/.test(path)) for (const m of bare(text).matchAll(/(?:^|[\s{;'"`])(?:style:)?(--fk-[\w-]+)\s*[:=]/g)) declared.add(m[1]!)

  it('declares every token that a var() uses without a fallback', () => {
    const missing: string[] = []
    for (const { path, text } of sources)
      for (const m of text.matchAll(/var\(\s*(--fk-[\w-]+)\s*\)/g)) if (!declared.has(m[1]!)) missing.push(`${path}: ${m[1]}`)
    expect(missing).toEqual([])
  })

  it('does not lean on a fallback for a token that is declared nowhere', () => {
    const missing: string[] = []
    for (const { path, text } of sources)
      for (const m of text.matchAll(/var\(\s*(--fk-[\w-]+)\s*,/g)) if (!declared.has(m[1]!)) missing.push(`${path}: ${m[1]}`)
    expect(missing).toEqual([])
  })

  it('sees the radii that were missing', () => {
    expect(declared.has('--fk-radius-panel')).toBe(true)
    expect(declared.has('--fk-radius')).toBe(false)
    expect(declared.has('--fk-radius-sm')).toBe(false)
  })
})
