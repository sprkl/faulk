import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { CORE_SECONDS, DURATIONS_FILE, SERVER_SECONDS, SETUP_SECONDS, checkDurations, costUnits, planShards, reportTests, specFile, specNames, testKey } from './e2e-plan.mjs'

// The machines of a run, dealt out by measured time (#848): the planner, the measures file, the CLI of each machine
// and the refresh of the measures.

const ROOT = fileURLToPath(new URL('../../', import.meta.url))
const E2E = join(ROOT, 'web/tests/e2e/')
const EXISTING = specNames(readdirSync(E2E))
const SHARD_CLI = fileURLToPath(new URL('./e2e-shard.mjs', import.meta.url))
const DURATIONS_CLI = fileURLToPath(new URL('./e2e-durations.mjs', import.meta.url))

/** A Playwright JSON report: `tests` is [project, spec, describes, title, ms?]. */
function report(tests) {
  const files = new Map()
  for (const [project, spec, describes, title, ms] of tests) {
    const file = `${spec}.spec.ts`
    if (!files.has(file)) files.set(file, { title: file, file, specs: [], suites: [] })
    let suite = files.get(file)
    for (const d of describes) {
      let child = suite.suites.find((s) => s.title === d)
      if (!child) suite.suites.push((child = { title: d, file, specs: [], suites: [] }))
      suite = child
    }
    let spec2 = suite.specs.find((s) => s.title === title)
    if (!spec2) suite.specs.push((spec2 = { title, file, tests: [] }))
    spec2.tests.push({ projectName: project, status: 'expected', results: ms === undefined ? [] : [{ duration: ms, status: 'passed' }] })
  }
  return { suites: [...files.values()] }
}

const ids = (machine) => machine.units.map((u) => `${u.project}/${u.spec}`)

test('planShards puts every unit on exactly one machine, the same way every time', () => {
  const units = []
  for (const project of ['dev', 'preview']) for (let i = 0; i < 40; i++) units.push({ project, spec: `s${String(i).padStart(2, '0')}`, seconds: (i * 37) % 61 })
  for (const shards of [1, 2, 3, 4, 7]) {
    const plan = planShards(units, shards)
    assert.equal(plan.length, shards)
    const all = plan.flatMap(ids)
    assert.equal(all.length, units.length, `${shards} machines`)
    assert.deepEqual([...all].sort(), units.map((u) => `${u.project}/${u.spec}`).sort())
    assert.deepEqual(planShards([...units].reverse(), shards), plan, 'the input order does not matter')
  }
  assert.deepEqual(planShards([], 3).map((m) => m.units.length), [0, 0, 0])
})

test('planShards balances by time, not by count, and charges a server once per project and machine', () => {
  const units = [
    { project: 'dev', spec: 'big', seconds: 600 },
    ...Array.from({ length: 30 }, (_, i) => ({ project: 'dev', spec: `small${i}`, seconds: 20 })),
  ]
  const plan = planShards(units, 3)
  // The big spec alone on a machine; the small ones on the other two.
  const big = plan.find((m) => ids(m).includes('dev/big'))
  assert.deepEqual(ids(big), ['dev/big'])
  for (const m of plan) assert.ok(m.seconds <= 600 + SERVER_SECONDS.dev, `${m.seconds}`)
  // Count-based slicing would have put 10 or 11 specs on each machine.
  assert.equal(plan.reduce((a, m) => a + m.units.length, 0), 31)
  // A project's server is charged once on a machine.
  const one = planShards([{ project: 'dev', spec: 'a', seconds: 10 }, { project: 'dev', spec: 'b', seconds: 10 }, { project: 'preview', spec: 'a', seconds: 10 }], 1)
  assert.equal(one[0].seconds, 30 + SERVER_SECONDS.dev + SERVER_SECONDS.preview)
})

test('planShards: the preload (the core run) weighs on the first machine; bad input is refused', () => {
  const units = Array.from({ length: 6 }, (_, i) => ({ project: 'dev', spec: `s${i}`, seconds: 30 }))
  const plan = planShards(units, 2, { preload: [CORE_SECONDS] })
  assert.ok(plan[0].units.length < plan[1].units.length, JSON.stringify(plan))
  assert.throws(() => planShards(units, 0), /machines/)
  assert.throws(() => planShards([...units, units[0]], 2), /twice/)
  assert.throws(() => planShards([{ project: 'dev', spec: 'x', seconds: Number.NaN }], 2), /costs/)
})

test('costUnits sums the listed tests, drops @full ones from a trimmed spec, adds the set-up, prices the unknown at the median', () => {
  const tests = reportTests(
    report([
      ['dev', 'a', [], 'one', undefined],
      ['dev', 'a', ['group'], 'two @full', undefined],
      ['dev', 'b', [], 'new test', undefined],
      ['dev', 'agent-scenario', [], 'scenario', undefined],
    ]),
  )
  const durations = { dev: { [testKey('a.spec.ts', ['one'])]: 3, [testKey('a.spec.ts', ['group', 'two @full'])]: 50, 'x.spec.ts › y': 7, 'agent-scenario.spec.ts › scenario': 6 } }
  const by = (units) => Object.fromEntries(units.map((u) => [u.spec, u.seconds]))
  // The median of 3, 50, 7, 6 (sorted 3, 6, 7, 50) is 7.
  assert.deepEqual(by(costUnits(tests, durations)), { a: 53, b: 7, 'agent-scenario': 6 + SETUP_SECONDS['agent-scenario'] })
  assert.deepEqual(by(costUnits(tests, durations, new Set(['a']))), { a: 3, b: 7, 'agent-scenario': 6 + SETUP_SECONDS['agent-scenario'] })
  // No measures at all: 2 s a test.
  assert.equal(costUnits(tests, {})[0].seconds, 4)
})

test('reportTests keys a test by its file, describes and title, as the list reporter prints them', () => {
  const t = reportTests(report([['preview', 'isolation', ['the landing page'], '/fr works', 1500]]))
  assert.deepEqual(t, [{ project: 'preview', spec: 'isolation', key: 'isolation.spec.ts › the landing page › /fr works', seconds: 1.5, status: 'expected' }])
  assert.throws(() => reportTests({ suites: [{ title: 'x.ts', file: 'x.ts', specs: [{ title: 't', file: 'x.ts', tests: [] }] }] }), /outside a spec/)
})

test('the measures file is well formed (a stale key costs nothing: no listed test reads it, and a refresh drops it)', () => {
  const durations = checkDurations(JSON.parse(readFileSync(join(ROOT, DURATIONS_FILE), 'utf8')))
  assert.ok(Object.keys(durations.dev).length > 300)
  assert.ok(Object.keys(durations.preview).length > 50)
  assert.throws(() => checkDurations({ dev: { 'not a key': 1 } }), /not a test key/)
  assert.throws(() => checkDurations({ dev: { 'a.spec.ts › t': -1 } }), /-1 s/)
  assert.throws(() => checkDurations({ dev: [] }), /not an object/)
})

test('a spec path given to Playwright matches its own file and no other (positional filters are regular expressions)', () => {
  for (const a of EXISTING) {
    const re = new RegExp(specFile(a), 'i')
    for (const b of EXISTING) {
      for (const path of [`/home/runner/work/faulk/faulk/web/${specFile(b)}`, `C:/git/faulk/web/${specFile(b)}`]) {
        assert.equal(re.test(path), a === b, `${specFile(a)} against ${path}`)
      }
    }
  }
})

// The preview subset (founder's decision of 2026-11-12) is written once, in playwright.config.ts.
const CONFIG = readFileSync(join(ROOT, 'web/playwright.config.ts'), 'utf8')
const PREVIEW_SPECS = [...(/export const PREVIEW_SPECS = \[([^\]]*)\]/.exec(CONFIG)?.[1] ?? '').matchAll(/'([a-z0-9-]+)'/g)].map((m) => m[1])
const PREVIEW_MIXER_TEST = /export const PREVIEW_MIXER_TEST = '([^']+)'/.exec(CONFIG)?.[1]

test('the preview subset names real specs, cites the decision, and its mixer test is one real test', () => {
  assert.ok(PREVIEW_SPECS.length >= 15, PREVIEW_SPECS.join())
  for (const s of PREVIEW_SPECS) assert.ok(EXISTING.includes(s), `${s} is no spec`)
  assert.deepEqual(PREVIEW_SPECS, [...new Set(PREVIEW_SPECS)].sort())
  assert.ok(!PREVIEW_SPECS.includes('mixer'), 'mixer runs one test on preview, through PREVIEW_MIXER_TEST')
  assert.match(CONFIG, /founder's decision of 2026-11-12/)
  // The bundle-sensitive specs of the audit stay: the engine and its worklet, isolation, storage, exports.
  for (const s of ['engine-mo2', 'engine-mo5', 'isolation', 'browser-storage', 'project-store', 'export-audio', 'landing-links', 'journey']) assert.ok(PREVIEW_SPECS.includes(s), s)
  const mixer = readFileSync(join(E2E, 'mixer.spec.ts'), 'utf8')
  const titles = [...mixer.matchAll(/\btest\(\s*'([^']*)'/g)].map((m) => m[1])
  assert.equal(titles.filter((t) => t.includes(PREVIEW_MIXER_TEST)).length, 1, `"${PREVIEW_MIXER_TEST}" is one test of mixer.spec.ts`)
})

// ---- the CLI of a machine ----

function shardRun(listing, args) {
  const dir = mkdtempSync(join(tmpdir(), 'e2e-shard-'))
  try {
    const file = join(dir, 'listing.json')
    writeFileSync(file, JSON.stringify(listing))
    const out = join(dir, 'out')
    writeFileSync(out, '')
    const r = spawnSync(process.execPath, [SHARD_CLI, '--listing', file, ...args], { encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: out, GITHUB_STEP_SUMMARY: '' } })
    const outputs = Object.fromEntries(
      readFileSync(out, 'utf8')
        .split('\n')
        .filter(Boolean)
        .map((l) => [l.slice(0, l.indexOf('=')), l.slice(l.indexOf('=') + 1)]),
    )
    return { r, outputs }
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

test('the machines of a run share the listed files out: each (project, file) once, no file left', () => {
  const listing = report(EXISTING.flatMap((s) => [['dev', s, [], 't', undefined], ...(s.startsWith('p') ? [['preview', s, [], 't', undefined]] : [])]))
  const seen = []
  for (let i = 1; i <= 3; i++) {
    const { r, outputs } = shardRun(listing, ['--shard', `${i}/3`])
    assert.equal(r.status, 0, r.stderr)
    for (const key of ['dev_trimmed', 'dev_whole', 'preview_trimmed', 'preview_whole', 'projects']) assert.ok(key in outputs, key)
    assert.equal(outputs.dev_trimmed, '')
    for (const p of ['dev', 'preview']) for (const f of outputs[`${p}_whole`].split(' ').filter(Boolean)) seen.push(`${p} ${f}`)
    assert.deepEqual(outputs.projects.split(' ').filter(Boolean), ['dev', 'preview'].filter((p) => outputs[`${p}_whole`]))
  }
  const expected = [...EXISTING.map((s) => `dev ${specFile(s)}`), ...EXISTING.filter((s) => s.startsWith('p')).map((s) => `preview ${specFile(s)}`)]
  assert.deepEqual(seen.sort(), expected.sort())
})

test('an area run: --trimmed specs are listed apart from the whole ones, and the core weighs on the first machine', () => {
  const listing = report([
    ['dev', 'tracks', [], 'a', undefined],
    ['dev', 'mixer', [], 'b', undefined],
    ['dev', 'grid', [], 'c', undefined],
  ])
  const outs = [1, 2].map((i) => shardRun(listing, ['--shard', `${i}/2`, '--trimmed', 'tests/e2e/tracks.spec.ts tests/e2e/grid.spec.ts', '--core']).outputs)
  const trimmed = outs.flatMap((o) => o.dev_trimmed.split(' ').filter(Boolean)).sort()
  const whole = outs.flatMap((o) => o.dev_whole.split(' ').filter(Boolean)).sort()
  assert.deepEqual(trimmed, ['tests/e2e/grid.spec.ts', 'tests/e2e/tracks.spec.ts'])
  assert.deepEqual(whole, ['tests/e2e/mixer.spec.ts'])
  // The core's 70 s on machine 1: the second machine takes more files.
  const count = (o) => [o.dev_trimmed, o.dev_whole].join(' ').split(' ').filter(Boolean).length
  assert.ok(count(outs[0]) < count(outs[1]), JSON.stringify(outs))
})

test('the CLI refuses a listing with no test, a bad shard and a missing listing', () => {
  assert.notEqual(shardRun({ suites: [] }, ['--shard', '1/3']).r.status, 0)
  assert.notEqual(shardRun(report([['dev', 'a', [], 't']]), ['--shard', '4/3']).r.status, 0)
  assert.notEqual(shardRun(report([['dev', 'a', [], 't']]), []).r.status, 0)
  assert.notEqual(spawnSync(process.execPath, [SHARD_CLI, '--shard', '1/3'], { encoding: 'utf8' }).status, 0)
})

// ---- the refresh of the measures ----

test('e2e-durations.mjs merges a run into the measures, keeps what the run did not measure, drops gone specs', () => {
  const dir = mkdtempSync(join(tmpdir(), 'e2e-durations-'))
  try {
    const table = join(dir, 'table.json')
    const real = EXISTING[0]
    writeFileSync(table, JSON.stringify({ '//': ['comment'], dev: { [`${real}.spec.ts › old`]: 9, [`${real}.spec.ts › kept`]: 4, 'no-such-spec-x.spec.ts › t': 1 } }))
    writeFileSync(
      join(dir, 'r.json'),
      JSON.stringify(
        report([
          ['dev', real, [], 'old', 1234],
          ['preview', real, ['d'], 'new', 50],
          ['preview', real, [], 'listed only', undefined],
        ]),
      ),
    )
    const r = spawnSync(process.execPath, [DURATIONS_CLI, join(dir, 'r.json'), '--table', table], { encoding: 'utf8' })
    assert.equal(r.status, 0, r.stderr)
    const next = JSON.parse(readFileSync(table, 'utf8'))
    assert.deepEqual(next, { '//': ['comment'], dev: { [`${real}.spec.ts › kept`]: 4, [`${real}.spec.ts › old`]: 1.2 }, preview: { [`${real}.spec.ts › d › new`]: 0.1 } })
    // --replace keeps the reports alone.
    assert.equal(spawnSync(process.execPath, [DURATIONS_CLI, dir, '--table', table, '--replace'], { encoding: 'utf8' }).status, 0)
    assert.deepEqual(JSON.parse(readFileSync(table, 'utf8')).dev, { [`${real}.spec.ts › old`]: 1.2 })
    // A report without a run (a listing) is no measure.
    writeFileSync(join(dir, 'r.json'), JSON.stringify(report([['dev', real, [], 'x', undefined]])))
    assert.notEqual(spawnSync(process.execPath, [DURATIONS_CLI, join(dir, 'r.json'), '--table', table], { encoding: 'utf8' }).status, 0)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
