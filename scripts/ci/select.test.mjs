import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'

const script = fileURLToPath(new URL('./select.mjs', import.meta.url))

function run(args, extraEnv = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'select-'))
  const out = join(dir, 'output')
  writeFileSync(out, '')
  const r = spawnSync(process.execPath, [script, ...args], { encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: out, ...extraEnv } })
  const lines = readFileSync(out, 'utf8').split('\n').filter(Boolean)
  const outputs = Object.fromEntries(lines.map((l) => [l.slice(0, l.indexOf('=')), l.slice(l.indexOf('=') + 1)]))
  return { r, outputs }
}

test('writes key=value lines to $GITHUB_OUTPUT (docs only, no cargo needed)', () => {
  const { r, outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files', 'docs/a.md'])
  assert.equal(r.status, 0)
  assert.equal(outputs.mode, 'targeted')
  assert.equal(outputs.web, 'false')
  assert.equal(outputs.e2e, '')
})

test('a push runs nothing, a release pull request everything', () => {
  assert.equal(run(['--event', 'push']).outputs.mode, 'none')
  const full = run(['--event', 'pull_request', '--base', 'main', '--files', 'docs/a.md']).outputs
  assert.equal(full.mode, 'full')
  assert.equal(full.e2e, 'dev,preview')
})

test('an unreadable workspace runs everything', () => {
  // With no PATH, cargo cannot be started.
  const { r, outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files', 'crates/helper/a.rs'], { PATH: '', Path: '' })
  assert.equal(r.status, 0)
  assert.equal(outputs.mode, 'full')
  assert.equal(outputs.helper_windows, 'true')
  assert.equal(outputs.rust_wasm_engine, 'true')
})

test('an unreadable file list runs everything', () => {
  const { outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files-from', join(tmpdir(), 'does-not-exist-select')])
  assert.equal(outputs.mode, 'full')
})

test('a view change writes the targeted end-to-end plan: its specs, the smoke tests, one shard (#377)', () => {
  const { r, outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files', 'web/src/views/tracks/TrackList.svelte'])
  assert.equal(r.status, 0)
  assert.equal(outputs.e2e, 'dev')
  assert.equal(outputs.e2e_all, 'false')
  assert.equal(outputs.e2e_specs, 'tests/e2e/track-order.spec.ts tests/e2e/tracks.spec.ts')
  assert.ok(outputs.e2e_smoke.includes('tests/e2e/export-audio.spec.ts') && outputs.e2e_smoke_grep.includes('export-audio'))
  assert.equal(outputs.e2e_shards, '[1]')
  assert.equal(outputs.e2e_shard_total, '1')
})

test('a shared web file, the release and a failure to list the files run every spec on three machines (#377)', () => {
  for (const args of [
    ['--base', 'develop', '--files', 'web/src/lib/stores/dispatcher.ts'],
    ['--base', 'main', '--files', 'docs/a.md'],
    ['--base', 'develop', '--files-from', join(tmpdir(), 'does-not-exist-select')],
  ]) {
    const { outputs } = run(['--event', 'pull_request', ...args])
    assert.equal(outputs.e2e_all, 'true', args.join(' '))
    assert.equal(outputs.e2e_specs, '', args.join(' '))
    assert.equal(outputs.e2e_shards, '[1,2,3]', args.join(' '))
    assert.equal(outputs.e2e_shard_total, '3', args.join(' '))
  }
})

test('no browser test to run: no end-to-end project, one placeholder shard behind the job condition (#377)', () => {
  const { outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files', 'web/src/lib/a.test.ts'])
  assert.equal(outputs.web, 'true')
  assert.equal(outputs.e2e, '')
  assert.equal(outputs.e2e_all, 'false')
  assert.equal(outputs.e2e_shards, '[1]')
})
