import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { ALL, MAX_SHARDS, NONE, RULES, SMOKE, SPECS_PER_SHARD, needsWeb, planE2e, selectSpecs, shardCount, smokeBySpec, smokeGrep, specNames } from './e2e-plan.mjs'

const E2E = fileURLToPath(new URL('../../web/tests/e2e/', import.meta.url))
const EXISTING = specNames(readdirSync(E2E))
const plan = (files, options) => planE2e(files, EXISTING, options)
const source = (spec) => readFileSync(`${E2E}${spec}.spec.ts`, 'utf8')

// The specs that no area rule names: they run when their own file changes and on every whole-suite
// run, and nowhere else. They cover the engine, the dispatcher, the stores and the shell, which the
// table sends to the whole suite anyway. A new spec must either be named by an area rule or be added
// here on purpose: this list is the enumeration that proves "every spec is placed".
const WHOLE_SUITE_ONLY = ['engine-channel', 'engine-mo2', 'engine-mo5', 'refused-commands', 'stop-to-marker', 'value-entry']

test('every spec on disk is named by an area rule or deliberately whole-suite only', () => {
  const named = new Set()
  for (const [, what] of RULES) if (Array.isArray(what)) what.forEach((n) => named.add(n))
  const unplaced = EXISTING.filter((n) => !named.has(n))
  assert.deepEqual(unplaced, [...WHOLE_SUITE_ONLY].sort(), 'a spec is neither in the table nor in WHOLE_SUITE_ONLY')
})

test('every spec name the table lists exists on disk', () => {
  for (const [, what] of RULES) if (Array.isArray(what)) for (const n of what) assert.ok(EXISTING.includes(n), `the table names "${n}": no such spec`)
})

test('every spec on disk runs when its own file changes', () => {
  for (const name of EXISTING) assert.deepEqual(plan([`web/tests/e2e/${name}.spec.ts`]).specs, [name])
})

test('every rule is either a verdict or a list of specs', () => {
  for (const [re, what] of RULES) assert.ok(re instanceof RegExp && (what === ALL || what === NONE || Array.isArray(what) || typeof what === 'function'), String(re))
})

test('the smoke set: real specs, real tests, the four backbone areas, no duplicates', () => {
  assert.ok(SMOKE.length >= 4)
  const subst = (s) => s.replaceAll('${lang}', 'en').replaceAll('${width}', '1280')
  for (const { spec, title } of SMOKE) {
    assert.ok(EXISTING.includes(spec), `smoke names "${spec}": no such spec`)
    assert.equal(subst(source(spec)).split(title).length - 1, 1, `smoke test "${title}" must appear once in ${spec}.spec.ts`)
  }
  assert.equal(new Set(SMOKE.map((s) => `${s.spec}|${s.title}`)).size, SMOKE.length)
  const areas = { 'audio start': 'isolation', transport: 'transport', 'save and reopen': 'project-store', 'audio export': 'export-audio' }
  for (const [area, spec] of Object.entries(areas)) assert.ok(smokeBySpec()[spec], area)
})

test('the smoke grep matches the printed title of its tests and not their neighbours', () => {
  const re = new RegExp(smokeGrep(['transport']))
  assert.ok(re.test('[dev] › tests/e2e/transport.spec.ts:69:5 › transport, en, 1280 px: play, stop, tempo, signature, loop, metronome'))
  assert.ok(!re.test('[dev] › tests/e2e/transport.spec.ts:69:5 › transport, fr, 1280 px: play, stop, tempo, signature, loop, metronome'))
  assert.ok(!re.test('[dev] › tests/e2e/other.spec.ts:1:1 › transport, en, 1280 px: play, stop, tempo, signature, loop, metronome'))
  assert.equal(smokeGrep([]), '')
  // A title with regular-expression characters stays literal.
  assert.ok(new RegExp(smokeGrep(['export-audio'])).test('x export-audio.spec.ts:137:1 › exports the song as a 24-bit WAV file while the song plays: header'))
})

test('one change, one plan: whole specs, smoke of the others, shards', () => {
  const p = plan(['web/src/views/mixer/MixerFader.svelte'])
  assert.deepEqual(p.specs, ['automation', 'continuous', 'master-chain', 'master-pan', 'mixer', 'mixer-sound', 'track-state'])
  assert.deepEqual(p.smoke, ['export-audio', 'isolation', 'open-project', 'project-store', 'transport'])
  assert.equal(p.all, false)
  assert.equal(p.run, true)
  assert.equal(p.shards, 2)
  assert.ok(p.grep.length > 0)
})

test('nothing a browser sees: no plan to run, no smoke either', () => {
  for (const f of ['docs/a.md', 'scripts/agent/verify.mjs', 'web/src/lib/a.test.ts', 'web/README.md', 'web/.prettierrc.json', 'web/.prettierignore', 'web/eslint.config.js', 'CLAUDE.md', '.claude/settings.json', '.github/ISSUE_TEMPLATE/x.yml']) {
    const p = plan([f])
    assert.deepEqual([p.run, p.specs, p.smoke, p.shards], [false, [], [], 0], f)
  }
  assert.deepEqual(plan([]).run, false)
})

test('the files that decide what runs run the whole suite', () => {
  for (const f of ['.github/workflows/ci.yml', '.github/actions/web-setup/action.yml', 'scripts/ci/e2e-plan.mjs', 'scripts/ci/select-core.mjs']) {
    const p = plan([f])
    assert.equal(p.all, true, f)
    assert.equal(needsWeb([f]), true, f)
  }
  assert.equal(needsWeb(['.github/workflows/rust.yml']), false)
})

test('shared files and stale tables fall back to the whole suite on three machines', () => {
  const shared = ['web/src/lib/stores/dispatcher.ts', 'web/src/views/shell/AppShell.svelte', 'web/src/lib/engine/worklet.ts', 'web/src/lib/generated/commands.ts', 'web/src/lib/i18n/app.ts', 'web/tests/e2e/support.ts', 'web/tests/e2e/fixtures/measure-rpc.js', 'web/package.json', 'web/pnpm-lock.yaml', 'crates/engine/src/lib.rs', 'crates/protocol/src/lib.rs', 'Cargo.lock', 'web/some-new-folder/x.ts', 'brand-new-top-folder/x']
  for (const f of shared) {
    const p = plan(['web/src/views/tracks/T.svelte', f])
    assert.equal(p.all, true, f)
    assert.deepEqual(p.specs, EXISTING, f)
    assert.deepEqual([p.smoke, p.shards], [[], MAX_SHARDS], f)
  }
  const stale = planE2e(['web/src/views/mixer/MixerView.svelte'], ['mixer'])
  assert.deepEqual([stale.all, stale.shards], [true, MAX_SHARDS])
  const unreadable = planE2e(['web/src/views/mixer/MixerView.svelte'], [])
  assert.deepEqual([unreadable.run, unreadable.all, unreadable.shards], [true, true, MAX_SHARDS])
  assert.equal(plan(['docs/a.md'], { all: true }).all, true) // forced by the caller
})

test('shards: one for a short list, up to three, never zero for a run that happens', () => {
  assert.equal(shardCount(1), 1)
  assert.equal(shardCount(SPECS_PER_SHARD), 1)
  assert.equal(shardCount(SPECS_PER_SHARD + 1), 2)
  assert.equal(shardCount(2 * SPECS_PER_SHARD + 1), 3)
  assert.equal(shardCount(500), MAX_SHARDS)
  for (const f of ['web/src/views/arrangement/A.svelte', 'web/src/views/grid/G.svelte', 'web/src/lib/storage/s.ts', 'web/src/lib/export/e.ts', 'web/src/views/tracks/T.svelte']) {
    const p = plan([f])
    assert.ok(p.shards >= 1 && p.shards <= MAX_SHARDS, f)
    assert.equal(p.all, false, f)
  }
})

test('smoke titles stay quotable on a Windows command line (verify.mjs wraps the grep in double quotes)', () => {
  for (const { title } of SMOKE) assert.ok(!/["%^&<>!]/.test(title), title)
})

test('the shared time grid reaches every area that draws it (the piano roll as well as the arrangement)', () => {
  const p = plan(['web/src/lib/components/timegrid/TimeGrid.svelte'])
  for (const n of ['timegrid', 'arrangement-view', 'piano-roll', 'piano-roll-edit', 'piano-roll-jump']) assert.ok(p.specs.includes(n), n)
})

test('selectSpecs stays the same pure function verify.mjs always used', () => {
  assert.deepEqual(selectSpecs(['web/tests/e2e/piano-roll.spec.ts'], EXISTING).specs, ['piano-roll'])
})
