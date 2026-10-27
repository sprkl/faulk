import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, it } from 'node:test'
import { fileURLToPath } from 'node:url'
import { checkoutOf, editionOf, extensionOf, formatSteps } from './format-core.mjs'

const root = join(tmpdir(), 'faulk-checkout')
const everything = () => true
const nothing = () => false
const plan = (rel, exists = everything) => formatSteps({ file: join(root, ...rel.split('/')), root, exists, node: 'node', edition: '2024' })
const names = (rel, exists) => plan(rel, exists).map((s) => s.name)

describe('formatSteps', () => {
  it('lints then formats the TypeScript and Svelte files of web/', () => {
    assert.deepEqual(names('web/src/lib/a.ts'), ['eslint', 'prettier'])
    assert.deepEqual(names('web/src/views/A.svelte'), ['eslint', 'prettier'])
    assert.deepEqual(names('web/src/lib/state.svelte.ts'), ['eslint', 'prettier'])
    assert.deepEqual(names('web/scripts/build.mjs'), ['eslint', 'prettier'])
    assert.deepEqual(names('web/tests/e2e/x.spec.ts'), ['eslint', 'prettier'])
  })

  it('only formats what ESLint does not read', () => {
    assert.deepEqual(names('web/package.json'), ['prettier'])
    assert.deepEqual(names('web/src/app.css'), ['prettier'])
  })

  it('runs both from web/, on the file, with the checkout own tools', () => {
    const [eslint, prettier] = plan('web/src/a.ts')
    assert.equal(prettier.cwd, join(root, 'web'))
    assert.equal(prettier.args[0], join(root, 'web', 'node_modules', 'prettier', 'bin', 'prettier.cjs'))
    assert.equal(prettier.args.at(-1), join(root, 'web', 'src', 'a.ts'))
    assert.equal(eslint.args[0], join(root, 'web', 'node_modules', 'eslint', 'bin', 'eslint.js'))
    assert.ok(eslint.args.includes('--fix'))
  })

  it('formats Rust with rustfmt and the edition of the workspace', () => {
    const [step] = plan('crates/engine/src/lib.rs')
    assert.equal(step.command, 'rustfmt')
    assert.deepEqual(step.args.slice(0, 2), ['--edition', '2024'])
    assert.equal(step.cwd, root)
    assert.equal(plan('crates/engine/src/lib.rs').length, 1)
  })

  it('does nothing when the web tools are not installed', () => {
    assert.deepEqual(plan('web/src/a.ts', nothing), [])
  })

  it('leaves documentation, configuration and unknown files alone', () => {
    for (const rel of ['docs/agents/verification.md', 'README.md', '.github/workflows/ci.yml', 'Cargo.toml', 'web/pnpm-lock.yaml', 'web/static/a.png', 'scripts/agent/verify.mjs']) {
      assert.deepEqual(plan(rel), [], rel)
    }
  })

  it('never touches dependencies, build output or generated code', () => {
    for (const rel of [
      'web/node_modules/x/index.js',
      'web/src/lib/generated/protocol.ts',
      'web/src/lib/engine/wasm/engine.js',
      'web/.svelte-kit/a.ts',
      'web/dist/a.js',
      'crates/engine/target/debug/build/x.rs',
    ]) {
      assert.deepEqual(plan(rel), [], rel)
    }
  })

  it('refuses a file outside the checkout, however it is spelled', () => {
    const other = join(tmpdir(), 'other-checkout', 'web', 'src', 'a.ts')
    assert.deepEqual(formatSteps({ file: other, root, exists: everything }), [])
    assert.deepEqual(formatSteps({ file: join(root, 'web', '..', '..', 'x', 'a.ts'), root, exists: everything }), [])
  })
})

describe('helpers', () => {
  it('extensionOf', () => {
    assert.equal(extensionOf(join('a', 'B.SVELTE')), '.svelte')
    assert.equal(extensionOf(join('a', 'state.svelte.ts')), '.ts')
    assert.equal(extensionOf(join('a', '.gitignore')), '')
    assert.equal(extensionOf(join('a', 'Makefile')), '')
  })

  it('checkoutOf finds the closest .git, a file in a worktree as well', () => {
    const exists = (p) => p === join(root, '.git')
    assert.equal(checkoutOf(join(root, 'web', 'src', 'a.ts'), exists), root)
    assert.equal(checkoutOf(join(root, 'web', 'src', 'a.ts'), nothing), null)
  })

  it('editionOf reads the workspace edition and defaults to 2024', () => {
    assert.equal(editionOf('[workspace.package]\nedition = "2021"\n'), '2021')
    assert.equal(editionOf('[workspace]\nmembers = []\n'), '2024')
  })
})

describe('the hook', () => {
  const hook = fileURLToPath(new URL('./format-hook.mjs', import.meta.url))
  const run = (input, args = []) => spawnSync(process.execPath, [hook, ...args], { input, encoding: 'utf8', timeout: 30_000 })

  it('exits 0 and prints nothing on garbage, an empty payload, a missing file or a file outside any checkout', () => {
    const dir = mkdtempSync(join(tmpdir(), 'faulk-hook-'))
    try {
      const file = join(dir, 'a.ts')
      writeFileSync(file, 'export const a   =  1;;\n')
      for (const input of ['', 'not json', '{}', '{"tool_input":{}}', JSON.stringify({ tool_input: { file_path: join(dir, 'missing.ts') } }), JSON.stringify({ tool_input: { file_path: file } })]) {
        const result = run(input)
        assert.equal(result.status, 0, input)
        assert.equal(result.stdout, '')
        assert.equal(result.stderr, '')
      }
      assert.equal(readFileSync(file, 'utf8'), 'export const a   =  1;;\n') // no checkout around it: untouched
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('exits 0 in a checkout whose tools are not installed, and leaves the file as it is', () => {
    const dir = mkdtempSync(join(tmpdir(), 'faulk-hook-'))
    try {
      mkdirSync(join(dir, '.git'))
      mkdirSync(join(dir, 'web', 'src'), { recursive: true })
      const file = join(dir, 'web', 'src', 'a.ts')
      writeFileSync(file, 'export const a   =  1;;\n')
      const result = run(JSON.stringify({ tool_input: { file_path: file } }))
      assert.equal(result.status, 0)
      assert.equal(result.stdout, '')
      assert.equal(readFileSync(file, 'utf8'), 'export const a   =  1;;\n')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
