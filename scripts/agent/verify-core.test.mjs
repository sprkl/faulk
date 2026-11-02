import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { RULES, dirSizeUpTo, duration, freePorts, needsWeb, parsePorts, portIsFree, selectSpecs, sharedTargetDir, specNames, summary } from './verify-core.mjs'

const E2E = fileURLToPath(new URL('../../web/tests/e2e/', import.meta.url))
const EXISTING = specNames(readdirSync(E2E))
const pick = (files) => selectSpecs(files, EXISTING)

test('the specs on disk are read from the real directory', () => {
  assert.ok(EXISTING.length > 30)
  assert.ok(EXISTING.includes('piano-roll') && EXISTING.includes('isolation'))
})

test('a changed spec runs itself and nothing else', () => {
  const r = pick(['web/tests/e2e/piano-roll.spec.ts'])
  assert.deepEqual(r.specs, ['piano-roll'])
  assert.equal(r.all, false)
})

test('a view maps to its specs', () => {
  const r = pick(['web/src/views/mixer/MixerFader.svelte'])
  assert.deepEqual(r.specs, ['automation', 'continuous', 'layout-4', 'master-chain', 'master-pan', 'mixer', 'mixer-sound', 'track-state'])
  assert.equal(r.all, false)
})

test('several files add up, a shared file widens to the whole suite', () => {
  assert.deepEqual(pick(['web/src/views/grid/GridView.svelte', 'web/src/views/tracks/TrackList.svelte']).specs, ['clip-names', 'grid', 'grid-clips', 'grid-to-arrangement', 'lane-rows', 'recording', 'track-order', 'tracks'])
  for (const shared of ['web/src/lib/stores/dispatcher.ts', 'web/src/lib/i18n/app.ts', 'web/tests/e2e/support.ts', 'web/package.json', 'crates/engine/src/lib.rs', 'Cargo.lock', 'web/src/lib/engine/worklet.ts']) {
    const r = pick(['web/src/views/mixer/MixerFader.svelte', shared])
    assert.equal(r.all, true, shared)
    assert.deepEqual(r.specs, EXISTING, shared)
  }
})

test('docs, scripts, unit tests and notes need no browser test', () => {
  const r = pick(['docs/agents/verification.md', 'scripts/agent/verify.mjs', 'web/src/lib/keyboard.test.ts', 'web/README.md', 'CLAUDE.md', '.claude/settings.json'])
  assert.deepEqual(r.specs, [])
  assert.equal(r.all, false)
  assert.equal(needsWeb(['docs/x.md', 'scripts/a.mjs', '.github/workflows/rust.yml']), false)
})

test('a unit test inside tests/e2e is a spec change, not a note', () => {
  assert.deepEqual(pick(['web/tests/e2e/support.ts']).all, true)
})

test('an unknown path, a deleted spec or a stale table never skips a test', () => {
  const unknown = pick(['some-new-top-folder/file.txt'])
  assert.equal(unknown.all, true)
  assert.deepEqual(unknown.reasons.unknown, ['some-new-top-folder/file.txt'])
  const deleted = selectSpecs(['web/tests/e2e/gone.spec.ts'], EXISTING)
  assert.equal(deleted.all, true)
  assert.equal(deleted.reasons.missing.length, 1)
  assert.equal(selectSpecs(['web/src/views/mixer/MixerView.svelte'], ['mixer']).all, true) // table names specs that do not exist
})

test('Windows separators are understood', () => {
  assert.deepEqual(pick(['web\\src\\views\\tracks\\TrackList.svelte']).specs, ['lane-rows', 'track-order', 'tracks'])
})

test('every spec name the table lists exists on disk', () => {
  const named = new Set()
  for (const [re, what] of RULES) if (Array.isArray(what)) what.forEach((n) => named.add(n))
  for (const n of named) assert.ok(EXISTING.includes(n), `the table names "${n}": no such spec`)
})

test('every spec on disk is reachable: it runs when its own file changes', () => {
  for (const name of EXISTING) assert.deepEqual(pick([`web/tests/e2e/${name}.spec.ts`]).specs, [name])
})

test('the web checks run unless every file is one no browser test sees; an unknown path runs them', () => {
  for (const f of ['web/src/a.ts', 'crates/commands/src/lib.rs', 'Cargo.toml', 'modules/x/a.dsp', '.cargo/config.toml', 'some-new-top-folder/f', 'helper/x.rs']) assert.equal(needsWeb([f]), true, f)
  assert.equal(needsWeb(['docs/a.md', 'web/src/a.test.ts', '.github/workflows/rust.yml']), false)
  assert.equal(needsWeb(['.github/workflows/ci.yml']), true) // the workflow that runs the browser tests
  assert.equal(needsWeb([]), false)
})

test('three distinct free ports, usable afterwards', async () => {
  const ports = await freePorts(3)
  assert.equal(new Set(ports).size, 3)
  for (const p of ports) assert.equal(await portIsFree(p), true)
})

test('a port in use is reported busy', async () => {
  const server = createServer()
  await new Promise((r) => server.listen(0, '127.0.0.1', r))
  const { port } = server.address()
  assert.equal(await portIsFree(port), false)
  await new Promise((r) => server.close(r))
  assert.equal(await portIsFree(port), true)
})

test('--ports wants three distinct valid ports', () => {
  assert.deepEqual(parsePorts('5304, 4304,47904'), [5304, 4304, 47904])
  for (const bad of ['1,2', '1,1,2', 'a,b,c', '0,1,2', '1,2,70000', '']) assert.throws(() => parsePorts(bad), /--ports/, bad)
})

test('the build cache directory follows the platform', () => {
  assert.equal(sharedTargetDir({ LOCALAPPDATA: 'C:\\Users\\a\\AppData\\Local' }, 'win32', 'C:\\Users\\a'), 'C:\\Users\\a\\AppData\\Local\\faulk-build\\target')
  assert.equal(sharedTargetDir({}, 'win32', 'C:\\Users\\a'), 'C:\\Users\\a\\AppData\\Local\\faulk-build\\target')
  assert.equal(sharedTargetDir({ XDG_CACHE_HOME: '/c' }, 'linux', '/home/a'), '/c/faulk-build/target')
  assert.equal(sharedTargetDir({}, 'linux', '/home/a'), '/home/a/.cache/faulk-build/target')
  assert.equal(sharedTargetDir({}, 'linux', ''), null)
})

test('the size of a tree is counted, and counting stops past the limit', () => {
  const dir = mkdtempSync(join(tmpdir(), 'faulk-size-'))
  try {
    mkdirSync(join(dir, 'a', 'b'), { recursive: true })
    writeFileSync(join(dir, 'x'), Buffer.alloc(100))
    writeFileSync(join(dir, 'a', 'b', 'y'), Buffer.alloc(300))
    assert.equal(dirSizeUpTo(dir, 1e9), 400)
    assert.ok(dirSizeUpTo(dir, 50) > 50)
    assert.equal(dirSizeUpTo(join(dir, 'missing'), 10), 0)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('durations read short', () => {
  assert.equal(duration(12340), '12.3 s')
  assert.equal(duration(125000), '2 min 05 s')
  assert.equal(duration(119800), '2 min 00 s')
})

test('the summary names the failure', () => {
  const ok = { name: 'lint', ok: true, ms: 1000 }
  const bad = { name: 'vitest', ok: false, ms: 2000 }
  const text = summary([ok, bad], { base: 'origin/develop', files: ['a'], specs: ['mixer'], all: false })
  assert.match(text, /FAIL vitest/)
  assert.match(text, /RESULT: FAIL \(vitest\)/)
  assert.match(text, /e2e specs: mixer/)
  const fine = summary([ok, { name: 'web checks', ok: true, skipped: true, ms: 0 }], { base: 'b', files: [], specs: [], all: false })
  assert.match(fine, /RESULT: OK in 1\.0 s/)
  assert.match(fine, /none needed/)
})
