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
  // A task pull request: the whole dev suite, not the release's two projects (#848).
  assert.equal(outputs.e2e, 'dev')
})

test('an unreadable file list runs everything', () => {
  const { outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files-from', join(tmpdir(), 'does-not-exist-select')])
  assert.equal(outputs.mode, 'full')
})

test('a view change writes the area tier: its specs, the core tests, one shard (#377, #533)', () => {
  const { r, outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files', 'web/src/views/tracks/TrackList.svelte'])
  assert.equal(r.status, 0)
  assert.equal(outputs.e2e, 'dev')
  assert.equal(outputs.e2e_tier, 'area')
  assert.match(outputs.e2e_why, /area of: web\/src\/views\/tracks\/TrackList\.svelte/)
  assert.equal(outputs.e2e_all, 'false')
  assert.equal(
    outputs.e2e_specs,
    'tests/e2e/device-browser.spec.ts tests/e2e/discoverability.spec.ts tests/e2e/lane-rows.spec.ts tests/e2e/track-order.spec.ts tests/e2e/tracks.spec.ts',
  )
  assert.ok(outputs.e2e_core.includes('tests/e2e/export-audio.spec.ts') && outputs.e2e_core_grep.includes('export-audio'))
  assert.equal(outputs.e2e_shards, '[1]')
  assert.equal(outputs.e2e_shard_total, '1')
})

test('a shared page file writes the core tier: no area spec, the core tests, one shard (#533)', () => {
  const { outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files', 'web/src/lib/stores/dispatcher.ts'])
  assert.deepEqual([outputs.e2e, outputs.e2e_tier, outputs.e2e_all, outputs.e2e_specs, outputs.e2e_shards], ['dev', 'core', 'false', '', '[1]'])
  assert.ok(outputs.e2e_core.includes('tests/e2e/helper.spec.ts'))
  assert.match(outputs.e2e_why, /shared page file/)
})

test('a contract, the release and a failure to list the files run every spec on four machines (#377)', () => {
  for (const args of [
    ['--base', 'develop', '--files', 'web/src/lib/generated/protocol.ts'],
    ['--base', 'main', '--files', 'docs/a.md'],
    ['--base', 'develop', '--files-from', join(tmpdir(), 'does-not-exist-select')],
  ]) {
    const { outputs } = run(['--event', 'pull_request', ...args])
    assert.equal(outputs.e2e_all, 'true', args.join(' '))
    assert.equal(outputs.e2e_specs, '', args.join(' '))
    assert.equal(outputs.e2e_shards, '[1,2,3,4]', args.join(' '))
    assert.equal(outputs.e2e_shard_total, '4', args.join(' '))
  }
})

test('no browser test to run: no end-to-end project, one placeholder shard behind the job condition (#377)', () => {
  const { outputs } = run(['--event', 'pull_request', '--base', 'develop', '--files', 'web/src/lib/a.test.ts'])
  assert.equal(outputs.web, 'true')
  assert.equal(outputs.e2e, '')
  assert.equal(outputs.e2e_all, 'false')
  assert.equal(outputs.e2e_shards, '[1]')
})
