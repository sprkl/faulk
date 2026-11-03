import assert from 'node:assert/strict'
import { readdirSync } from 'node:fs'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { planE2e, specNames } from '../ci/e2e-plan.mjs'
import { evidenceLine, planVerify, rootSteps } from './verify-plan.mjs'

// A miniature of the real workspace (names, dependency edges, dev-only edges).
const WS = [
  { name: 'engine', dir: 'crates/engine', deps: [], devDeps: [] },
  { name: 'project', dir: 'crates/project', deps: ['engine'], devDeps: [] },
  { name: 'commands', dir: 'crates/commands', deps: ['engine', 'project'], devDeps: [] },
  { name: 'protocol', dir: 'crates/protocol', deps: ['engine', 'commands'], devDeps: [] },
  { name: 'engine-wasm', dir: 'crates/engine-wasm', deps: ['engine'], devDeps: [] },
  { name: 'project-wasm', dir: 'crates/project-wasm', deps: ['project', 'commands'], devDeps: [] },
  { name: 'helper', dir: 'crates/helper', deps: ['engine', 'project', 'commands', 'protocol'], devDeps: [] },
  { name: 'test-support', dir: 'crates/test-support', deps: [], devDeps: ['engine', 'project', 'commands'] },
]
// The specs on disk: the core names several of them, and a core spec that is missing is a stale table (the whole suite).
const SPECS = specNames(readdirSync(fileURLToPath(new URL('../../web/tests/e2e/', import.meta.url))))
const plan = (files, extra = {}) => planVerify({ files, packages: () => WS, specs: () => SPECS, ...extra })
const tested = (p) => p.decision.rustTest.split(' ').filter((x) => x !== '-p')

test('a change to a leaf crate tests that crate only', () => {
  const p = plan(['crates/test-support/src/lib.rs'])
  assert.equal(p.mode, 'targeted')
  assert.deepEqual(tested(p), ['test-support'])
  assert.equal(p.decision.rustWasmEngine, false)
})

test('reverse dependencies inside the workspace are tested, dev-only users included', () => {
  const p = plan(['crates/commands/src/lib.rs'])
  assert.deepEqual(tested(p).sort(), ['commands', 'helper', 'project-wasm', 'protocol', 'test-support'])
})

test('the parity suites (WASM builds) run only when the engine or what embeds it changes', () => {
  const eng = plan(['crates/engine/src/lib.rs'])
  assert.equal(eng.decision.rustWasmEngine, true)
  assert.equal(eng.decision.rustWasmProject, true)
  assert.equal(eng.decision.rustTest, '--workspace') // every crate depends on the engine, or dev-depends on it
  const helper = plan(['crates/helper/src/main.rs'])
  assert.deepEqual(tested(helper), ['helper'])
  assert.equal(helper.decision.rustWasmEngine, false)
  assert.equal(helper.decision.rustWasmProject, false)
  assert.equal(helper.decision.web, false)
  assert.deepEqual(rootSteps(helper.decision).map((s) => s.name), ['fmt', 'clippy', 'cargo test'])
})

test('the web part is the e2e plan of the CI', () => {
  const files = ['web/src/views/mixer/MixerFader.svelte']
  const p = plan(files)
  assert.equal(p.decision.web, true)
  const e = planE2e(files, SPECS)
  assert.deepEqual([p.decision.e2eSpecs, p.decision.e2eCore, p.decision.e2eGrep, p.decision.e2eTier, p.decision.e2eWhy], [e.specs, e.core, e.grep, e.tier, e.why])
  assert.equal(p.decision.e2eAll, false)
  // A shared page file: the core tests alone, as in CI.
  const shared = plan(['web/src/lib/stores/dispatcher.ts']).decision
  assert.deepEqual([shared.e2eTier, shared.e2eSpecs, shared.e2eAll], ['core', [], false])
  assert.deepEqual(plan(['docs/x.md']).decision.web, false)
})

test('--full, unknown files, a shared file and an unreadable workspace run everything', () => {
  for (const p of [plan(['docs/x.md'], { full: true }), plan(null), plan(['unknown-folder/a.txt']), plan(['scripts/ci/select-core.mjs'])]) {
    assert.equal(p.mode, 'full')
    assert.equal(p.decision.rustTest, '--workspace')
    assert.equal(p.decision.e2eAll, true)
  }
  const broken = planVerify({
    files: ['crates/engine/src/lib.rs'],
    packages: () => {
      throw new Error('no cargo')
    },
    specs: () => SPECS,
  })
  assert.equal(broken.mode, 'full')
  assert.match(broken.why, /unreadable/)
  assert.equal(planVerify({ files: ['crates/engine/src/lib.rs'], packages: () => [], specs: () => SPECS }).mode, 'full') // empty workspace
})

test('nothing to run for a docs-only change; scripts run the agent scripts tests; the engine runs the proto crate', () => {
  assert.deepEqual(rootSteps(plan(['docs/a.md']).decision), [])
  assert.deepEqual(rootSteps(plan(['scripts/agent/x.mjs']).decision).map((s) => s.name), ['agent scripts'])
  const names = rootSteps(plan(['crates/engine/src/lib.rs']).decision).map((s) => s.name)
  assert.ok(names.includes('proto fmt') && names.includes('wasm project'))
  assert.ok(rootSteps(plan(['crates/engine/src/lib.rs']).decision).filter((s) => s.cwd).every((s) => s.cwd === 'web/src/lib/engine/proto/crate'))
})

test('the evidence line names the mode and what ran', () => {
  const line = evidenceLine({ mode: 'targeted', base: 'origin/develop', decision: plan(['crates/helper/src/main.rs']).decision })
  assert.match(line, /targeted vs origin\/develop/)
  assert.match(line, /cargo test -p helper/)
  assert.match(line, /wasm builds none/)
})

test("a build input change tests the whole workspace and the whole suite", () => {
  const p = plan(["Cargo.toml"])
  assert.equal(p.decision.rustTest, "--workspace")
  assert.equal(p.decision.e2eAll, true)
  assert.equal(p.decision.rustWasmEngine, true)
})
