// The ESLint guard of #436: a TypeScript or Svelte file of `src/` must not import the 42 MB schema bundle (svelte-check runs out of memory on its inferred type).
import { ESLint } from 'eslint'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const web = fileURLToPath(new URL('..', import.meta.url))
const eslint = new ESLint({ cwd: web })

/** The lint messages of `code` as if it were the file `path` (relative to `web/`). */
async function lint(code: string, path: string): Promise<string[]> {
  const [result] = await eslint.lintText(code, { filePath: join(web, path) })
  // A parse error would hide the rule: fail on it rather than let the rejection tests pass by accident.
  const fatal = result.messages.find((message) => message.fatal)
  if (fatal) throw new Error(fatal.message)
  return result.messages.map((message) => message.message)
}

const rejected = (messages: string[]) => messages.filter((message) => message.includes('generated/schema.json'))

describe('lint guard on generated/schema.json (#436)', () => {
  it.each([
    ['a static import', `import schema from '../lib/generated/schema.json'\nexport const s = schema\n`],
    ['a relative import from the root of src', `import schema from './lib/generated/schema.json'\nexport const s = schema\n`],
    ['a side-effect import', `import '../lib/generated/schema.json'\n`],
    ['a named re-export', `export { default as schema } from '../lib/generated/schema.json'\n`],
    ['a re-export of everything', `export * from '../lib/generated/schema.json'\n`],
    ['a dynamic import', `export const load = () => import('../lib/generated/schema.json')\n`],
  ])('rejects %s in a TypeScript file of src', async (_, code) => {
    expect(rejected(await lint(code, 'src/lib/keyboard.ts'))).toHaveLength(1)
  })

  it('rejects the import in a Svelte file of src', async () => {
    const code = `<script lang="ts">\n  import schema from '../generated/schema.json'\n  const s = schema\n</script>\n{s}\n`
    expect(rejected(await lint(code, 'src/lib/components/Knob.svelte'))).toHaveLength(1)
  })

  it.each(['src/lib/engine/proto/clock-worker.js', 'src/lib/export/audio-export.svelte.ts'])('rejects the import in %s', async (path) => {
    const code = `import schema from '../generated/schema.json'\nexport const s = schema\n`
    expect(rejected(await lint(code, path))).toHaveLength(1)
  })

  it('accepts the generated types and other JSON', async () => {
    const code = `import type { Command } from '../generated/protocol'\nimport pkg from '../../package.json' with { type: 'json' }\nexport const x: Command | undefined = undefined\nexport const v = pkg\n`
    expect(rejected(await lint(code, 'src/lib/keyboard.ts'))).toHaveLength(0)
  })

  it.each([
    ['a type-position import type', `export type Schema = typeof import('../lib/generated/schema.json')\n`],
    ['an import equals of require', `import schema = require('../lib/generated/schema.json')\nexport const s = schema\n`],
    ['a require call', `export const schema = require('../lib/generated/schema.json')\n`],
  ])('rejects %s', async (_, code) => {
    expect(rejected(await lint(code, 'src/lib/keyboard.ts'))).toHaveLength(1)
  })

  it('guards the tests and the scripts too, which svelte-check also types', async () => {
    const code = `import schema from '../src/lib/generated/schema.json'\nexport const s = schema\n`
    expect(rejected(await lint(code, 'tests/e2e/navigation.spec.ts'))).toHaveLength(1)
    expect(rejected(await lint(code, 'scripts/schema-import-guard.test.ts'))).toHaveLength(1)
  })

  it.each(['scripts/gen-protocol.mjs', 'scripts/schema-merge.mjs'])('rejects the import in %s', async (path) => {
    const code = `import schema from '../src/lib/generated/schema.json' with { type: 'json' }\nexport const s = schema\n`
    expect(rejected(await lint(code, path))).toHaveLength(1)
  })

  it('does not reject a runtime read of the file', async () => {
    const code = `import { readFileSync } from 'node:fs'\nexport const s = JSON.parse(readFileSync(new URL('../lib/generated/schema.json', import.meta.url), 'utf8'))\n`
    expect(rejected(await lint(code, 'src/lib/keyboard.ts'))).toHaveLength(0)
  })
})
