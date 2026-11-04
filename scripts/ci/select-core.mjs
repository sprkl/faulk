// Decides which CI jobs and steps a change needs. Pure: no I/O, so the rules are testable.
//
// Policy (issue #330):
//  - push (develop, main): nothing heavy, the code was tested in its pull request;
//  - pull request against `main` (release, hotfix): the full suite, once;
//  - pull request against another branch: only what the changed files can affect;
//  - the browser tests of a pull request against another branch follow the tiers of the one table
//    shared with scripts/agent/verify.mjs (e2e-plan.mjs, #377, #533): area specs plus the core tests,
//    the core tests alone for a shared page file, the whole suite for a contract or a configuration;
//  - fail safe: a file this module cannot classify, an unknown event or a failure to list the
//    changed files runs everything.

import { FULL_TAG, MAX_SHARDS, TABLE_FILE, explain, planE2e, tableOnlyChange } from './e2e-plan.mjs'

const NONE = Object.freeze({
  mode: 'none',
  rustChecks: false, // cargo fmt + clippy on the workspace
  rustTest: '', // arguments of `cargo test`: '' (no test), '--workspace' or '-p a -p b'
  rustWasmEngine: false, // wasm builds of engine-wasm
  rustWasmProject: false, // wasm build of project-wasm
  rustProto: false, // fmt + clippy of the prototype crate outside the workspace
  helperWindows: false,
  web: false,
  e2e: '', // '', 'dev' or 'dev,preview' (the Playwright projects)
  e2eTier: 'none', // 'none', 'core' (the core tests alone), 'area' (area specs plus the core tests) or 'all'
  e2eWhy: '', // one line: which files chose the tier
  e2eAll: false, // the whole suite (every spec of the projects), not a list
  e2eSpecs: [], // spec names run whole but for the @full tests (targeted only)
  e2eChanged: [], // specs whose own file changed: run whole, @full tests included (targeted only)
  e2eCore: [], // specs of which only the core tests run (targeted only), on the first machine
  e2eGrep: '', // the --grep that selects those core tests
  e2eShards: 0, // machines to run them on; 0: no end-to-end job
  agentScripts: false,
  packages: [],
})

// The crates whose code ends up in the web page (WASM builds) or in its generated types.
export const WEB_CRATES = ['engine-wasm', 'project-wasm', 'protocol', 'audio-import-wasm']
const HELPER = 'helper'
const HELPER_DIR = `crates/${HELPER}`
// The prototype crate (web/src/lib/engine/proto/crate) depends on these by path but is not a workspace member.
const PROTO_DEPS = ['engine']

// Files that need no job at all.
const IGNORED = [
  /^docs\//,
  /^\.scratch\//,
  /^[^/]+\.md$/,
  /^(LICENSE|\.gitignore|\.git-blame-ignore-revs)[^/]*$/,
  /^\.github\/(ISSUE_TEMPLATE\/|PULL_REQUEST_TEMPLATE|CODEOWNERS$|dependabot\.yml$|[^/]+\.md$)/, // not .github/actions/
]

/** Everything runs. Needs no workspace, so it also serves when the workspace cannot be read. */
export function fullDecision(workspace = [], why = 'everything runs') {
  return full(workspace, why)
}

function full(workspace, why) {
  return {
    mode: 'full',
    rustChecks: true,
    rustTest: '--workspace',
    rustWasmEngine: true,
    rustWasmProject: true,
    rustProto: true,
    helperWindows: true,
    web: true,
    e2e: 'dev,preview',
    e2eTier: 'all',
    e2eWhy: explain({ forced: [why] }),
    e2eAll: true,
    e2eSpecs: [],
    e2eChanged: [],
    e2eCore: [],
    e2eGrep: '',
    e2eShards: MAX_SHARDS,
    agentScripts: true,
    packages: workspace.map((p) => p.name),
  }
}

/**
 * What a change to `changed` packages affects. `built`: those packages and every package that
 * depends on one of them through a normal or build dependency, transitively (their code changes).
 * `tested`: `built` plus the packages that only dev-depend on one of `built` (their tests change,
 * what they ship does not).
 * @param {{name: string, dir: string, deps: string[], devDeps?: string[]}[]} packages
 * @param {Iterable<string>} changed package names
 */
export function affectedPackages(packages, changed) {
  const dependents = new Map(packages.map((p) => [p.name, []]))
  for (const p of packages) for (const d of p.deps) dependents.get(d)?.push(p.name)
  const built = new Set()
  const queue = [...changed]
  while (queue.length) {
    const name = queue.pop()
    if (built.has(name) || !dependents.has(name)) continue
    built.add(name)
    queue.push(...dependents.get(name))
  }
  const tested = new Set(built)
  for (const p of packages) if ((p.devDeps ?? []).some((d) => built.has(d))) tested.add(p.name)
  return { built, tested }
}

/**
 * @param {object} input
 * @param {string} input.event GitHub event name
 * @param {string} input.base base branch of the pull request
 * @param {string[]|null} input.files changed files (repo-relative), null when unknown
 * @param {() => {name: string, dir: string, deps: string[]}[]} input.packages the workspace crates,
 *   read lazily: a change without Rust never asks for them
 * @param {() => string[]} [input.specs] the end-to-end spec names on disk, read lazily; absent, empty
 *   or throwing: the whole suite
 * @param {() => {base: string, head: string, files: string[]}} [input.table] the e2e table file before and
 *   after the change and the repository's files, read lazily when it is the only file of scripts/ci/ that
 *   changed; absent or throwing: everything
 */
export function select({ event, base, files, packages, specs, table }) {
  if (event === 'push') return { ...NONE }
  if (event !== 'pull_request') return full(packages(), `event ${event || 'unknown'}`)
  if (base === 'main') return full(packages(), 'pull request against main (release or hotfix)')
  if (!Array.isArray(files)) return full(packages(), 'changed files unknown')

  const out = { ...NONE, mode: 'targeted' }
  const changedDirs = new Set()
  let web = false
  const e2eFiles = [] // what the e2e table judges: web files, ci.yml, build inputs, the files of crates the page is built from
  const crateFiles = new Map() // crate dir -> its changed files
  let tableChanged = false
  let tableSpecs = null // the table alone changed: the specs it newly names
  let rustAll = false
  let helperInputs = false
  let workspace = null
  const members = () => {
    workspace ??= packages()
    if (!workspace.length) throw new Error('empty cargo workspace')
    return workspace
  }

  for (const raw of files) {
    const f = raw.replaceAll('\\', '/')
    if (IGNORED.some((re) => re.test(f))) continue

    // The selection rules themselves: a mistake there would skip tests anywhere.
    // An edit of the end-to-end table alone is judged after the loop (tableOnlyChange); the rest of scripts/ci/ runs everything.
    if (f === TABLE_FILE) { tableChanged = true; continue }
    if (f.startsWith('scripts/ci/')) return full(members(), `the selection changed: ${f}`)
    if (f === '.github/workflows/ci.yml') { web = true; e2eFiles.push(f); continue }
    if (f === '.github/workflows/rust.yml') { rustAll = out.rustProto = true; continue }
    if (f === '.github/workflows/agent-scripts.yml') { out.agentScripts = true; continue }
    // It has its own trial build; the scripts tests check its macOS rule (#452).
    if (f === '.github/workflows/release-helper.yml') { out.agentScripts = true; continue }

    // Workspace-wide Rust inputs, and the future Faust / Rust modules.
    if (/^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml)$/.test(f) || f.startsWith('.cargo/') || f.startsWith('modules/')) {
      rustAll = out.rustProto = web = true
      e2eFiles.push(f)
      continue
    }
    const crate = /^crates\/([^/]+)\/./.exec(f)
    if (crate) {
      const dir = `crates/${crate[1]}`
      if (!members().some((p) => p.dir === dir)) return full(members(), `a crate cargo does not list: ${f}`)
      changedDirs.add(dir)
      if (!crateFiles.has(dir)) crateFiles.set(dir, [])
      crateFiles.get(dir).push(f)
      continue
    }
    if (f.startsWith('crates/')) return full(members(), `unknown path: ${f}`)

    if (f.startsWith('web/')) {
      web = true
      e2eFiles.push(f)
      if (f.startsWith('web/src/lib/engine/proto/crate/')) out.rustProto = true
      // Read by the helper's updater tests (and, for vercel.json, by the release script tests).
      if (f === 'web/vercel.json' || f === 'web/src/lib/i18n/helper.ts') helperInputs = true
      if (f === 'web/vercel.json') out.agentScripts = true
      // Adding, renaming or deleting a spec: the table's own tests must run (every spec placed in the table).
      if (f.startsWith('web/tests/e2e/')) out.agentScripts = true
      continue
    }
    if (f.startsWith('scripts/') || f.startsWith('.claude/')) { out.agentScripts = true; continue }
    return full(members(), `unknown path: ${f}`) // never skip on a guess
  }

  // The e2e table alone changed under scripts/ci/: its own tests (agent scripts), the core tests and the
  // specs it newly names; anything the strict check cannot prove a table edit runs everything.
  if (tableChanged) {
    let check
    try {
      const { base: before, head: after, files: repoFiles } = table()
      check = tableOnlyChange(before, after, repoFiles)
    } catch (e) {
      check = { ok: false, why: `the table file cannot be compared (${e.message})` }
    }
    if (!check.ok) return full(members(), `the selection changed: ${TABLE_FILE} (${check.why})`)
    tableSpecs = check.specs
    out.agentScripts = true
    web = true
  }

  if (changedDirs.size || rustAll || helperInputs) {
    const all = members()
    const names = all.filter((p) => changedDirs.has(p.dir)).map((p) => p.name)
    if (helperInputs) names.push(HELPER)
    const everything = new Set(all.map((p) => p.name))
    const { built, tested } = rustAll ? { built: everything, tested: everything } : affectedPackages(all, names)
    out.packages = all.map((p) => p.name).filter((n) => tested.has(n))
    out.rustChecks = changedDirs.size > 0 || rustAll
    out.rustTest = out.packages.length === all.length ? '--workspace' : out.packages.map((n) => `-p ${n}`).join(' ')
    // The WASM builds, the page and the prototype crate see a crate's code, not its tests.
    out.rustWasmEngine = built.has('engine-wasm')
    out.rustWasmProject = built.has('project-wasm')
    out.helperWindows = tested.has(HELPER)
    if (PROTO_DEPS.some((n) => built.has(n))) out.rustProto = true
    if (WEB_CRATES.some((n) => built.has(n))) web = true
    // The helper's own change reaches no page code, but the browser tests drive the real helper (the
    // agent scenario, AG-5; the stand-in of helper.spec.ts): its files go to the e2e table (#668).
    if (changedDirs.has(HELPER_DIR)) web = true
    // A crate whose own change reaches the page hands its files to the e2e table, which tells the
    // contracts (protocol, commands, engine sync: the whole suite) from the rest (an area plus the core).
    // The helper is the exception: the page talks to it over the link, so the table gives it the
    // `helper` area (#668). A crate that reaches neither (test-support) adds no browser test.
    for (const p of all) {
      const reachesPage = p.dir === HELPER_DIR || WEB_CRATES.some((n) => affectedPackages(all, [p.name]).built.has(n))
      if (changedDirs.has(p.dir) && reachesPage) e2eFiles.push(...crateFiles.get(p.dir))
    }
  }
  out.web = web
  if (e2eFiles.length || tableSpecs) {
    // The table decides per file; anything it cannot place, or specs it cannot read, is the whole suite.
    let existing = []
    try {
      existing = specs?.() ?? []
    } catch {
      /* unreadable: the whole suite */
    }
    const plan = planE2e(e2eFiles, existing, tableSpecs ? { table: { specs: tableSpecs } } : {})
    out.e2eTier = plan.tier
    out.e2eWhy = plan.why
    if (plan.run) {
      out.e2e = 'dev'
      out.e2eAll = plan.all
      out.e2eSpecs = plan.all ? [] : plan.specs
      out.e2eChanged = plan.all ? [] : plan.changed
      out.e2eCore = plan.core
      out.e2eGrep = plan.grep
      out.e2eShards = plan.shards
    }
  }
  return out
}

/** `cargo metadata --no-deps` JSON to the shape `select` uses. */
export function packagesFromMetadata(metadata) {
  const root = metadata.workspace_root.replaceAll('\\', '/').replace(/\/$/, '')
  const members = new Set(metadata.workspace_members)
  return metadata.packages
    .filter((p) => members.has(p.id))
    .map((p) => {
      const manifest = p.manifest_path.replaceAll('\\', '/')
      const dir = manifest.slice(root.length + 1).replace(/\/Cargo\.toml$/, '')
      const local = p.dependencies.filter((d) => d.path)
      const names = (dev) => [...new Set(local.filter((d) => (d.kind === 'dev') === dev).map((d) => d.name))]
      return { name: p.name, dir, deps: names(false), devDeps: names(true).filter((n) => !names(false).includes(n)) }
    })
}

/** One line on the end-to-end tier of a decision and why: the CI step summary and verify.mjs print it. */
export function e2eSummary(r) {
  const what = {
    none: 'no browser test',
    core: `the core tests alone (${r.e2eCore.length} specs)`,
    area: `${r.e2eSpecs.length + r.e2eChanged.length} area specs${r.e2eChanged.length ? ` (${r.e2eChanged.length} changed, run with their ${FULL_TAG} tests)` : ''}, plus the core tests of ${r.e2eCore.length} other specs`,
    all: `the whole suite, ${FULL_TAG} tests included`,
  }[r.e2eTier]
  const tag = r.e2eTier === 'area' || r.e2eTier === 'core' ? `; ${FULL_TAG} tests left to the whole suite` : ''
  return `e2e tier: ${r.e2eTier}, ${what}${tag}${r.e2eWhy ? ` (${r.e2eWhy})` : ''}`
}

/** Key / value pairs for $GITHUB_OUTPUT. */
export function toOutputs(r) {
  return {
    mode: r.mode,
    rust_checks: String(r.rustChecks),
    rust_test: r.rustTest,
    rust_wasm_engine: String(r.rustWasmEngine),
    rust_wasm_project: String(r.rustWasmProject),
    rust_proto: String(r.rustProto),
    helper_windows: String(r.helperWindows),
    web: String(r.web),
    e2e: r.e2e,
    e2e_tier: r.e2eTier,
    e2e_why: r.e2eWhy.replace(/[\r\n]+/g, ' '), // one line for $GITHUB_OUTPUT
    e2e_all: String(r.e2eAll),
    e2e_specs: r.e2eSpecs.map((n) => `tests/e2e/${n}.spec.ts`).join(' '), // names are [a-z0-9-]: safe to split
    e2e_specs_changed: r.e2eChanged.map((n) => `tests/e2e/${n}.spec.ts`).join(' '),
    e2e_full_tag: FULL_TAG,
    e2e_core: r.e2eCore.map((n) => `tests/e2e/${n}.spec.ts`).join(' '),
    e2e_core_grep: r.e2eGrep,
    e2e_shards: JSON.stringify(Array.from({ length: Math.max(r.e2eShards, 1) }, (_, i) => i + 1)), // the matrix; the job's `if` guards the empty case
    e2e_shard_total: String(Math.max(r.e2eShards, 1)),
    agent_scripts: String(r.agentScripts),
    rust: String(r.rustChecks || r.rustTest !== '' || r.rustProto),
  }
}
