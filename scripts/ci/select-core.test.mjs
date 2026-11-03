import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { CORE_TESTS, coreBySpec, specNames } from './e2e-plan.mjs'
import { affectedPackages, e2eSummary, fullDecision, packagesFromMetadata, select, toOutputs } from './select-core.mjs'

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

// Targeted end-to-end selection (#377), in tiers (#533).
const names = (r) => [...r.e2eSpecs].sort()
const CORE_SPECS = Object.keys(coreBySpec()).sort()

test('tier area: a view change runs its specs whole and the core tests of the others, on one machine', () => {
  const r = pr(['web/src/views/tracks/TrackList.svelte'])
  assert.equal(r.e2e, 'dev')
  assert.equal(r.e2eTier, 'area')
  assert.equal(r.e2eAll, false)
  assert.deepEqual(names(r), ['discoverability', 'lane-rows', 'track-order', 'tracks'])
  assert.deepEqual(r.e2eCore, CORE_SPECS)
  assert.equal(r.e2eShards, 1)
  const out = toOutputs(r)
  assert.equal(out.e2e_tier, 'area')
  assert.match(out.e2e_why, /area of: web\/src\/views\/tracks\/TrackList\.svelte/)
  assert.equal(out.e2e_all, 'false')
  assert.equal(out.e2e_specs, 'tests/e2e/discoverability.spec.ts tests/e2e/lane-rows.spec.ts tests/e2e/track-order.spec.ts tests/e2e/tracks.spec.ts')
  assert.equal(out.e2e_core.split(' ').length, CORE_SPECS.length)
  assert.equal(out.e2e_shards, '[1]')
  assert.match(e2eSummary(r), /^e2e tier: area, 4 area specs whole, plus the core tests of \d+ other specs \(area of: /)
})

test('a spec that is also a core spec runs whole, not twice', () => {
  const r = pr(['web/src/views/shell/ExportDialog.svelte'])
  assert.deepEqual(r.e2eSpecs, ['export-audio'])
  assert.ok(!r.e2eCore.includes('export-audio'))
})

test('tier core: a shared page file runs the core tests alone, on one machine', () => {
  for (const f of ['web/src/lib/stores/dispatcher.ts', 'web/src/views/shell/Shell.svelte', 'web/src/lib/i18n/app.ts', 'web/src/lib/styles/app.css']) {
    const r = pr([f])
    assert.deepEqual([r.e2e, r.e2eTier, r.e2eAll, r.e2eSpecs, r.e2eCore, r.e2eShards], ['dev', 'core', false, [], CORE_SPECS, 1], f)
    const out = toOutputs(r)
    assert.equal(out.e2e_specs, '', f)
    assert.ok(out.e2e_core_grep.length > 0, f)
    assert.match(e2eSummary(r), /^e2e tier: core, the core tests alone .*shared page file/, f)
  }
})

test('tier for crates: the contracts run all, the engine and the project an area, helper and test-support nothing', () => {
  for (const f of ['crates/protocol/src/lib.rs', 'crates/commands/src/track.rs', 'crates/engine/src/msg.rs', 'crates/engine/src/msg/codec.rs', 'crates/engine-wasm/src/lib.rs']) {
    const r = pr([f])
    assert.deepEqual([r.e2e, r.e2eTier, r.e2eAll, r.e2eShards], ['dev', 'all', true, 3], f)
    assert.match(r.e2eWhy, new RegExp(f.replaceAll('.', '\\.')), f)
  }
  const engine = pr(['crates/engine/src/devices/reverb.rs'])
  assert.equal(engine.e2eTier, 'area')
  for (const n of ['engine-channel', 'engine-mo2', 'mixer-sound', 'devices']) assert.ok(engine.e2eSpecs.includes(n), n)
  const project = pr(['crates/project-wasm/src/save.rs'])
  assert.equal(project.e2eTier, 'area')
  assert.ok(project.e2eSpecs.includes('project-store'))
  for (const f of ['crates/helper/src/tray.rs', 'crates/test-support/src/signal.rs']) assert.deepEqual([pr([f]).e2e, pr([f]).e2eTier], ['', 'none'], f)
  // helper with the engine: the engine's files decide, helper adds nothing.
  const both = pr(['crates/helper/src/tray.rs', 'crates/engine/src/devices/reverb.rs'])
  assert.deepEqual(names(both), names(engine))
  // A workspace-wide input or modules/: the whole suite.
  for (const f of ['Cargo.lock', 'modules/x/a.dsp']) assert.equal(pr([f]).e2eTier, 'all', f)
})

test('a longer list gets more machines, never more than three; the core tests do not add one', () => {
  assert.equal(pr(['web/src/views/arrangement/A.svelte']).e2eShards, 3) // 22 specs
  assert.equal(pr(['web/src/views/mixer/M.svelte']).e2eShards, 2) // 8 specs
  const many = pr(['web/src/views/arrangement/A.svelte', 'web/src/views/grid/G.svelte', 'web/src/views/mixer/M.svelte', 'web/src/views/shell/ProjectMenu.svelte'])
  assert.equal(many.e2eShards, 3)
  assert.ok(toOutputs(many).e2e_shards === '[1,2,3]')
})

test('no browser test to run: no end-to-end job, no shard', () => {
  for (const files of [['docs/a.md'], ['web/src/lib/a.test.ts'], ['web/README.md'], ['crates/helper/src/a.rs'], ['scripts/agent/verify.mjs']]) {
    const r = pr(files)
    assert.equal(r.e2e, '', files.join())
    assert.equal(r.e2eShards, 0, files.join())
    assert.deepEqual([r.e2eSpecs, r.e2eCore], [[], []], files.join())
    assert.match(e2eSummary(r), /^e2e tier: none/)
  }
})

test('tier all: contracts, configuration, unknown paths, a stale table and an unreadable spec list', () => {
  for (const f of ['web/src/lib/engine/worklet.ts', 'web/src/lib/generated/commands.ts', 'web/tests/e2e/support.ts', 'web/package.json', 'web/playwright.config.ts', '.github/workflows/ci.yml', 'Cargo.lock', 'crates/protocol/src/lib.rs', 'modules/x/a.dsp']) {
    const r = pr(['web/src/views/tracks/TrackList.svelte', f])
    assert.equal(r.e2eTier, 'all', f)
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

test('the release pull request and the fallbacks keep the full suite on three machines, and say why', () => {
  for (const r of [pr(['docs/a.md'], 'main'), pr(['web/src/views/tracks/T.svelte'], 'main'), fullDecision(), pr(['scripts/ci/x.mjs']), pr(['something-new/x'])]) {
    assert.equal(r.e2e, 'dev,preview')
    assert.equal(r.e2eTier, 'all')
    assert.equal(r.e2eAll, true)
    assert.equal(r.e2eShards, 3)
    assert.ok(r.e2eWhy.length > 0)
    const out = toOutputs(r)
    assert.deepEqual([out.e2e_specs, out.e2e_core, out.e2e_core_grep, out.e2e_shards], ['', '', '', '[1,2,3]'])
  }
  assert.match(pr(['docs/a.md'], 'main').e2eWhy, /pull request against main/)
  assert.match(pr(['something-new/x']).e2eWhy, /unknown path: something-new\/x/)
  assert.equal(pr(['web/src/views/tracks/T.svelte'], 'develop').e2e, 'dev') // only the release runs the production build
})

test('adding or deleting a spec runs the agent scripts, where the table is tested', () => {
  assert.equal(pr(['web/tests/e2e/brand-new.spec.ts']).agentScripts, true)
  assert.equal(pr(['web/tests/e2e/mixer.spec.ts']).agentScripts, true)
  assert.equal(pr(['web/src/views/mixer/M.svelte']).agentScripts, false)
})

test('the core grep names every core entry of the others and nothing else', () => {
  const r = pr(['web/src/views/tracks/T.svelte'])
  const entries = CORE_TESTS.filter((s) => r.e2eCore.includes(s.spec))
  // A whole-spec entry is one alternative, and makes the titles of its spec unnecessary.
  const whole = new Set(entries.filter((e) => e.title === undefined).map((e) => e.spec))
  const expected = whole.size + entries.filter((e) => e.title !== undefined && !whole.has(e.spec)).length
  // Split on the alternatives, not on the `(^|[ /])` anchor inside each one.
  assert.equal(r.e2eGrep.split(/\|(?!\[)/).length, expected)
})

test('every output the workflow reads is one select writes, and none of the old smoke names is left', () => {
  const yml = readFileSync(fileURLToPath(new URL('../../.github/workflows/ci.yml', import.meta.url)), 'utf8')
  const outputs = Object.keys(toOutputs(pr(['web/src/views/tracks/T.svelte'])))
  const read = [...yml.matchAll(/steps\.changes\.outputs\.([a-z_0-9]+)/g)].map((m) => m[1])
  for (const k of read) assert.ok(outputs.includes(k), `ci.yml reads ${k}, which select does not write`)
  for (const k of ['e2e_tier', 'e2e_why', 'e2e_core', 'e2e_core_grep', 'e2e_specs', 'e2e_all']) assert.ok(yml.includes(`needs.select.outputs.${k}`), `the e2e job does not use ${k}`)
  assert.ok(!/smoke/i.test(yml), 'ci.yml still names the smoke set')
  assert.match(yml, /\$E2E_CORE --grep "\$E2E_CORE_GREP"/)
})
