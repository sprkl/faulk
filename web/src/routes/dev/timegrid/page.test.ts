import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

// The time grid's test surface is emitted in every build but must hold nothing in production: the whole
// content sits behind `enabled`, which is true only on the dev server and in the e2e build.
describe('the /dev/timegrid test surface', () => {
  const source = readFileSync(fileURLToPath(new URL('./+page.svelte', import.meta.url)), 'utf8')
  const markup = source.slice(source.indexOf('</svelte:head>') + '</svelte:head>'.length, source.indexOf('<style>'))

  it('shows its content only when the dev flag is on', () => {
    expect(source).toMatch(/const enabled = import\.meta\.env\.DEV \|\| import\.meta\.env\.VITE_FAULK_DEV_ROUTES === '1'\r?\n/)
    expect(markup.trim().startsWith('{#if enabled}')).toBe(true)
    expect(markup.trim().endsWith('{/if}')).toBe(true)
    // One condition around everything: no second top-level block outside of it.
    expect(markup.match(/^\{#if /gm)).toHaveLength(1)
  })

  it('has no environment file that turns the flag on', () => {
    for (const name of ['.env', '.env.production', '.env.local']) {
      let text = ''
      try {
        text = readFileSync(fileURLToPath(new URL(`../../../../${name}`, import.meta.url)), 'utf8')
      } catch {
        // A missing file is the normal case.
      }
      expect(text, name).not.toContain('VITE_FAULK_DEV_ROUTES')
    }
  })
})
