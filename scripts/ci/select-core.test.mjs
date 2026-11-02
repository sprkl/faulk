import assert from 'node:assert/strict'
import { readdirSync } from 'node:fs'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { SMOKE, specNames } from './e2e-plan.mjs'
import { affectedPackages, fullDecision, packagesFromMetadata, select, toOutputs } from './select-core.mjs'

// The workspace graph of the repository (dev-dependencies included), as `cargo metadata` reports it.
const WORKSPACE = [
  { name: 'engine', dir: 'crates/engine', deps: [], devDeps: ['test-support', 'commands'] },
  { name: 'commands', dir: 'crates/commands', deps: ['engine', 'project'], devDeps: [] },
  { name: 'project', dir: 'crates/project', deps: ['engine'], devDeps: ['commands'] },
  { name: 'protocol', dir: 'crates/protocol', deps: ['commands', 'engine'], devDeps: ['project'] },
  { name: 'engine-wasm', dir: 'crates/engine-wasm', deps: ['engine'], devDeps: ['commands', 'test-support', 'project'] },
  { name: 'project-wasm', dir: 'crates/project-wasm', deps: ['project', 'commands', 'engine'], devDeps: [] },
  { name: 'helper', dir: 'crates/helper', deps: ['engine', 'commands', 'protocol', 'project'], devDeps: [] },
  { name: 'test-support', dir: 'crates/test-support', deps: ['engine', 'commands', 'project'], devDeps: [] },
]
const ws = () => WORKSPACE
const SPECS = specNames(readdirSync(fileURLToPath(new URL('../../web/tests/e2e/', import.meta.url))))
const specs = () => SPECS
const pr = (files, base = 'develop') => select({ event: 'pull_request', base, files, packages: ws, specs })

test('a push runs nothing heavy, on develop and on main', () => {
  const r = select({ event: 'push', base: '', files: ['crates/engine/src/lib.rs'], packages: ws })
  assert.equal(r.mode, 'none')
  assert.equal(r.web || r.rustChecks || r.helperWindows || r.agentScripts, false)
  assert.equal(r.rustTest, '')
  assert.equal(r.e2e, '')
})

test('the blame-ignore list runs nothing', () => {
  const r = pr(['.git-blame-ignore-revs'])
  assert.equal(r.mode, 'targeted')
  assert.equal(r.web || r.rustChecks || r.helperWindows || r.agentScripts || r.rustProto, false)
  assert.equal(r.e2e, '')
})

test('the formatter and lint settings run the web checks and no browser test', () => {
  for (const f of ['web/.prettierrc.json', 'web/.prettierignore', 'web/eslint.config.js']) {
    const r = pr([f])
    assert.equal(r.web, true, f)
    assert.equal(r.e2e, '', f)
    assert.equal(r.rustChecks, false, f)
  }
})

test('docs-only, notes and .scratch run nothing', () => {
  const r = pr(['docs/agents/verification.md', 'docs/log.d/330-x.md', 'README.md', 'CLAUDE.md', '.scratch/x.md', '.github/ISSUE_TEMPLATE/a.yml'])
  assert.equal(pr(['.github/actions/setup/action.yml']).mode, 'full')
  assert.equal(r.mode, 'targeted')
  assert.equal(toOutputs(r).rust, 'false')
  assert.equal(r.web || r.rustChecks || r.helperWindows || r.agentScripts || r.rustProto, false)
  assert.equal(r.rustTest, '')
  assert.equal(r.e2e, '')
})

test('web-only change: web job with the dev e2e project, no Rust', () => {
  const r = pr(['web/src/lib/mixer/meter.ts'])
  assert.equal(r.web, true)
  assert.equal(r.e2e, 'dev')
  assert.equal(r.rustChecks, false)
  assert.equal(r.rustTest, '')
  assert.equal(r.helperWindows, false)
})

test('a unit test or a note under web/ skips the e2e, an e2e spec does not', () => {
  assert.equal(pr(['web/src/lib/a.test.ts']).web, true)
  assert.equal(pr(['web/src/lib/a.test.ts']).e2e, '')
  assert.equal(pr(['web/README.md']).e2e, '')
  assert.equal(pr(['web/tests/e2e/mixer.spec.ts']).e2e, 'dev')
  assert.equal(pr(['web/src/lib/a.test.ts', 'web/src/lib/a.ts']).e2e, 'dev')
})

test('engine change: every dependent is tested, both wasm builds, the web job and helper', () => {
  const r = pr(['crates/engine/src/graph.rs'])
  assert.equal(r.rustTest, '--workspace')
  assert.equal(r.rustChecks, true)
  assert.equal(r.rustWasmEngine && r.rustWasmProject && r.helperWindows, true)
  assert.equal(r.web, true)
  assert.equal(r.e2e, 'dev')
})

test('helper-only change: helper tests, no wasm, no web, no e2e', () => {
  const r = pr(['crates/helper/src/tray.rs'])
  assert.equal(r.rustTest, '-p helper')
  assert.equal(r.rustChecks, true)
  assert.equal(r.helperWindows, true)
  assert.equal(r.rustWasmEngine || r.rustWasmProject, false)
  assert.equal(r.web, false)
  assert.equal(r.e2e, '')
})

test('a leaf crate that the page embeds runs the web job but not helper', () => {
  const r = pr(['crates/project-wasm/src/lib.rs'])
  assert.equal(r.rustTest, '-p project-wasm')
  assert.equal(r.rustWasmProject, true)
  assert.equal(r.rustWasmEngine, false)
  assert.equal(r.helperWindows, false)
  assert.equal(r.web, true)
})

test('protocol changes reach helper and the generated types of the page', () => {
  const r = pr(['crates/protocol/src/lib.rs'])
  assert.equal(r.rustTest, '-p protocol -p helper')
  assert.equal(r.web, true)
  assert.equal(r.helperWindows, true)
})

test('helper inputs outside crates/ run the helper tests; vercel.json also the script tests', () => {
  const r = pr(['web/src/lib/i18n/helper.ts'])
  assert.equal(r.rustTest, '-p helper')
  assert.equal(r.rustChecks, false)
  assert.equal(r.web, true)
  const v = pr(['web/vercel.json'])
  assert.equal(v.rustTest, '-p helper')
  assert.equal(v.agentScripts, true)
})

test('the prototype crate under web/ is linted by the Rust job', () => {
  const r = pr(['web/src/lib/engine/proto/crate/src/lib.rs'])
  assert.equal(r.rustProto, true)
  assert.equal(r.rustTest, '')
  assert.equal(r.web, true)
})

test('workspace-wide Rust inputs and modules/ run all of Rust and the web job', () => {
  for (const f of ['Cargo.lock', 'Cargo.toml', 'rust-toolchain.toml', '.cargo/config.toml', 'modules/reverb/src/lib.rs']) {
    const r = pr([f])
    assert.equal(r.rustTest, '--workspace', f)
    assert.equal(r.web && r.helperWindows && r.rustProto, true, f)
  }
})

test('scripts/ and .claude/ run only the agent scripts job', () => {
  for (const f of ['scripts/agent-qa/record.mjs', '.claude/settings.json', 'scripts/helper-release/publish.mjs']) {
    const r = pr([f])
    assert.equal(r.agentScripts, true, f)
    assert.equal(r.web || r.rustChecks || r.helperWindows, false, f)
  }
})

test('each workflow file runs the job it defines, and release-helper.yml runs only the scripts tests here', () => {
  assert.equal(pr(['.github/workflows/ci.yml']).web, true)
  assert.equal(pr(['.github/workflows/ci.yml']).e2e, 'dev')
  const rust = pr(['.github/workflows/rust.yml'])
  assert.equal(rust.rustTest, '--workspace')
  assert.equal(rust.helperWindows, true)
  assert.equal(pr(['.github/workflows/agent-scripts.yml']).agentScripts, true)
  assert.equal(pr(['.github/workflows/release-helper.yml']).web, false)
  assert.equal(pr(['.github/workflows/release-helper.yml']).agentScripts, true)
})

test('a change to the selection rules runs everything', () => {
  assert.equal(pr(['scripts/ci/select-core.mjs']).mode, 'full')
})

test('release or hotfix (base main): the full suite, dev and preview e2e, even for docs only', () => {
  for (const files of [['docs/x.md'], ['web/src/a.ts'], []]) {
    const r = pr(files, 'main')
    assert.equal(r.mode, 'full')
    assert.equal(r.e2e, 'dev,preview')
    assert.equal(r.rustTest, '--workspace')
    assert.equal(r.rustWasmEngine && r.rustWasmProject && r.helperWindows && r.web && r.agentScripts && r.rustProto, true)
  }
})

test('fail safe: unknown file, unlisted crate, unknown event and unreadable file list run everything', () => {
  assert.equal(pr(['something-new/file.txt']).mode, 'full')
  assert.equal(pr(['crates/newcrate/src/lib.rs']).mode, 'full')
  assert.equal(pr(['crates/loose-file.rs']).mode, 'full')
  assert.equal(select({ event: 'workflow_dispatch', base: '', files: [], packages: ws }).mode, 'full')
  assert.equal(select({ event: 'pull_request', base: 'develop', files: null, packages: ws }).mode, 'full')
})

test('Windows separators are normalised', () => {
  assert.equal(pr(['crates\\helper\\src\\tray.rs']).rustTest, '-p helper')
})

test('a mix takes the union of what each file needs', () => {
  const r = pr(['docs/a.md', 'crates/helper/src/a.rs', 'web/src/a.ts', 'scripts/x.mjs'])
  assert.equal(r.rustTest, '-p helper')
  assert.equal(r.web && r.agentScripts && r.helperWindows, true)
})

test('affectedPackages: normal dependents are rebuilt, dev-dependents only retested', () => {
  const g = [
    { name: 'a', dir: 'a', deps: [], devDeps: [] },
    { name: 'b', dir: 'b', deps: ['a'], devDeps: [] },
    { name: 'c', dir: 'c', deps: ['b'], devDeps: [] },
    { name: 'd', dir: 'd', deps: [], devDeps: ['c'] },
    { name: 'e', dir: 'e', deps: ['d'], devDeps: [] },
  ]
  const r = affectedPackages(g, ['a'])
  assert.deepEqual([...r.built].sort(), ['a', 'b', 'c'])
  assert.deepEqual([...r.tested].sort(), ['a', 'b', 'c', 'd'])
  assert.deepEqual([...affectedPackages(g, ['nope']).built], [])
})

test('a dev-only crate (test-support) retests its users without rebuilding the page or helper', () => {
  const r = pr(['crates/test-support/src/signal.rs'])
  assert.equal(r.rustTest, '-p engine -p engine-wasm -p test-support')
  assert.equal(r.web || r.helperWindows || r.rustWasmEngine || r.rustWasmProject || r.rustProto, false)
  assert.equal(r.e2e, '')
})

test('engine changes also lint the prototype crate that depends on it outside the workspace', () => {
  assert.equal(pr(['crates/engine/src/lib.rs']).rustProto, true)
  assert.equal(pr(['crates/helper/src/main.rs']).rustProto, false)
})

test('an empty workspace is an error (the CLI then runs everything), never a skip', () => {
  for (const f of ['Cargo.lock', 'web/vercel.json', 'web/src/lib/i18n/helper.ts', 'crates/helper/a.rs']) {
    assert.throws(() => select({ event: 'pull_request', base: 'develop', files: [f], packages: () => [] }), /empty cargo workspace/, f)
  }
  assert.equal(fullDecision().helperWindows, true)
  assert.equal(fullDecision().rustWasmEngine, true)
})

test('packagesFromMetadata keeps members and path dependencies only', () => {
  const pkgs = packagesFromMetadata({
    workspace_root: 'C:\\repo',
    workspace_members: ['m1', 'm2'],
    packages: [
      { id: 'm1', name: 'one', manifest_path: 'C:\\repo\\crates\\one\\Cargo.toml', dependencies: [{ name: 'two', path: 'C:\\repo\\crates\\two' }, { name: 'three', path: 'C:\repo\crates\three', kind: 'dev' }, { name: 'serde' }] },
      { id: 'm2', name: 'two', manifest_path: 'C:\\repo\\crates\\two\\Cargo.toml', dependencies: [] },
      { id: 'x', name: 'outside', manifest_path: 'C:\\other\\Cargo.toml', dependencies: [] },
    ],
  })
  assert.deepEqual(pkgs, [
    { name: 'one', dir: 'crates/one', deps: ['two'], devDeps: ['three'] },
    { name: 'two', dir: 'crates/two', deps: [], devDeps: [] },
  ])
})

// Targeted end-to-end selection (#377).
const names = (r) => [...r.e2eSpecs].sort()

test('a view change runs its specs whole and the smoke tests of the others, on one machine', () => {
  const r = pr(['web/src/views/tracks/TrackList.svelte'])
  assert.equal(r.e2e, 'dev')
  assert.equal(r.e2eAll, false)
  assert.deepEqual(names(r), ['discoverability', 'lane-rows', 'track-order', 'tracks'])
  assert.deepEqual(r.e2eSmoke, ['export-audio', 'isolation', 'open-project', 'project-store', 'transport'])
  assert.equal(r.e2eShards, 1)
  const out = toOutputs(r)
  assert.equal(out.e2e_all, 'false')
  assert.equal(out.e2e_specs, 'tests/e2e/discoverability.spec.ts tests/e2e/lane-rows.spec.ts tests/e2e/track-order.spec.ts tests/e2e/tracks.spec.ts')
  assert.equal(out.e2e_smoke.split(' ').length, 5)
  assert.equal(out.e2e_shards, '[1]')
})

test('a spec that is also a smoke spec runs whole, not twice', () => {
  const r = pr(['web/src/views/shell/ExportDialog.svelte'])
  assert.deepEqual(r.e2eSpecs, ['export-audio'])
  assert.ok(!r.e2eSmoke.includes('export-audio'))
})

test('a longer list gets more machines, never more than three', () => {
  assert.equal(pr(['web/src/views/arrangement/A.svelte']).e2eShards, 3) // 12 specs + the smoke tests
  const many = pr(['web/src/views/arrangement/A.svelte', 'web/src/views/grid/G.svelte', 'web/src/views/mixer/M.svelte', 'web/src/views/shell/ProjectMenu.svelte'])
  assert.equal(many.e2eShards, 3)
  assert.ok(toOutputs(many).e2e_shards === '[1,2,3]')
})

test('no browser test to run: no end-to-end job, no shard', () => {
  for (const files of [['docs/a.md'], ['web/src/lib/a.test.ts'], ['web/README.md'], ['crates/helper/src/a.rs'], ['scripts/agent/verify.mjs']]) {
    const r = pr(files)
    assert.equal(r.e2e, '', files.join())
    assert.equal(r.e2eShards, 0, files.join())
    assert.deepEqual([r.e2eSpecs, r.e2eSmoke], [[], []], files.join())
  }
})

test('shared files, unknown paths, a stale table and an unreadable spec list run the whole suite', () => {
  for (const f of ['web/src/lib/stores/dispatcher.ts', 'web/src/views/shell/AppShell.svelte', 'web/src/lib/engine/worklet.ts', 'web/src/lib/generated/commands.ts', 'web/tests/e2e/support.ts', 'web/package.json', 'web/playwright.config.ts', '.github/workflows/ci.yml', 'Cargo.lock', 'crates/engine/src/lib.rs', 'crates/protocol/src/lib.rs', 'modules/x/a.dsp']) {
    const r = pr(['web/src/views/tracks/TrackList.svelte', f])
    assert.equal(r.e2eAll, true, f)
    assert.equal(r.e2eShards, 3, f)
    assert.equal(toOutputs(r).e2e_specs, '', f)
  }
  assert.equal(select({ event: 'pull_request', base: 'develop', files: ['web/src/views/tracks/T.svelte'], packages: ws }).e2eAll, true) // no way to list the specs
  assert.equal(select({ event: 'pull_request', base: 'develop', files: ['web/src/views/tracks/T.svelte'], packages: ws, specs: () => [] }).e2eAll, true)
  assert.equal(select({ event: 'pull_request', base: 'develop', files: ['web/src/views/tracks/T.svelte'], packages: ws, specs: () => { throw new Error('nope') } }).e2eAll, true)
  assert.equal(select({ event: 'pull_request', base: 'develop', files: ['web/src/views/tracks/T.svelte'], packages: ws, specs: () => ['mixer'] }).e2eAll, true) // the table names specs that are gone
  assert.equal(pr(['web/src/views/tracks/T.svelte', 'web/brand-new-folder/x.ts']).e2eAll, true)
})

test('the release pull request and the fallbacks keep the full suite on three machines', () => {
  for (const r of [pr(['docs/a.md'], 'main'), pr(['web/src/views/tracks/T.svelte'], 'main'), fullDecision()]) {
    assert.equal(r.e2e, 'dev,preview')
    assert.equal(r.e2eAll, true)
    assert.equal(r.e2eShards, 3)
    const out = toOutputs(r)
    assert.deepEqual([out.e2e_specs, out.e2e_smoke, out.e2e_smoke_grep, out.e2e_shards], ['', '', '', '[1,2,3]'])
  }
  assert.equal(pr(['web/src/views/tracks/T.svelte'], 'develop').e2e, 'dev') // only the release runs the production build
})

test('adding or deleting a spec runs the agent scripts, where the table is tested', () => {
  assert.equal(pr(['web/tests/e2e/brand-new.spec.ts']).agentScripts, true)
  assert.equal(pr(['web/tests/e2e/mixer.spec.ts']).agentScripts, true)
  assert.equal(pr(['web/src/views/mixer/M.svelte']).agentScripts, false)
})

test('the smoke grep names every smoke test of the others and nothing else', () => {
  const r = pr(['web/src/views/tracks/T.svelte'])
  assert.equal(r.e2eGrep.split('|').length, SMOKE.filter((s) => r.e2eSmoke.includes(s.spec)).length)
})
