// The script itself, run as a process: its dry run reads the real repository, starts nothing.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./verify.mjs', import.meta.url))
const verify = (...args) => spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' })

test('a dry run against HEAD reads the changed files and starts nothing', () => {
  const r = verify('--dry-run', '--base', 'HEAD')
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /^base HEAD$/m)
  assert.match(r.stdout, /^files:$/m)
  assert.match(r.stdout, /specs \(\d+/)
})

test('--all selects the whole suite', () => {
  const r = verify('--dry-run', '--all', '--base', 'HEAD')
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /specs \(\d{2,}, whole suite\)/)
})

test('a base that cannot be read runs the whole suite and says so', () => {
  const r = verify('--dry-run', '--base', 'no-such-ref-372')
  assert.equal(r.status, 0, r.stderr)
  assert.match(r.stdout, /cannot compare with no-such-ref-372/)
  assert.match(r.stdout, /whole suite/)
  assert.match(r.stdout, /web checks: true/)
})

test('bad ports are refused before anything runs', () => {
  const r = verify('--dry-run', '--ports', '1,1,2')
  assert.equal(r.status, 2)
  assert.match(r.stderr, /--ports expects three distinct ports/)
})
