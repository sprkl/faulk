// Decides which CI jobs and steps a change needs. Pure: no I/O, so the rules are testable.
//
// Policy (issue #330):
//  - push (develop, main): nothing heavy, the code was tested in its pull request;
//  - pull request against `main` (release, hotfix): the full suite, once;
//  - pull request against another branch: only what the changed files can affect;
//  - the browser tests of a pull request against another branch are the specs its web files touch
//    plus a smoke set, from the one table shared with scripts/agent/verify.mjs (e2e-plan.mjs, #377);
//  - fail safe: a file this module cannot classify, an unknown event or a failure to list the
//    changed files runs everything.

import { MAX_SHARDS, planE2e } from './e2e-plan.mjs'

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
  e2eAll: false, // the whole suite (every spec of the projects), not a list
  e2eSpecs: [], // spec names run whole (targeted only)
  e2eSmoke: [], // specs of which only the smoke tests run (targeted only)
  e2eGrep: '', // the --grep that selects those smoke tests
  e2eShards: 0, // machines to run them on; 0: no end-to-end job
  agentScripts: false,
  packages: [],
})

// The crates whose code ends up in the web page (WASM builds) or in its generated types.
export const WEB_CRATES = ['engine-wasm', 'project-wasm', 'protocol']
const HELPER = 'helper'
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
export function fullDecision(workspace = []) {
  return full(workspace)
}

function full(workspace) {
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
    e2eAll: true,
    e2eSpecs: [],
    e2eSmoke: [],
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
 */
export function select({ event, base, files, packages, specs }) {
  if (event === 'push') return { ...NONE }
  if (event !== 'pull_request' || base === 'main' || !Array.isArray(files)) return full(packages())

  const out = { ...NONE, mode: 'targeted' }
  const changedDirs = new Set()
  let web = false
  let webE2eAll = false // something forces the whole suite (the workflow, the build inputs, a crate the page is built from)
  const webFiles = []
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
    if (f.startsWith('scripts/ci/')) return full(members())
    if (f === '.github/workflows/ci.yml') { web = webE2eAll = true; continue }
    if (f === '.github/workflows/rust.yml') { rustAll = out.rustProto = true; continue }
    if (f === '.github/workflows/agent-scripts.yml') { out.agentScripts = true; continue }
    // It has its own trial build; the scripts tests check its macOS rule (#452).
    if (f === '.github/workflows/release-helper.yml') { out.agentScripts = true; continue }

    // Workspace-wide Rust inputs, and the future Faust / Rust modules.
    if (/^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml)$/.test(f) || f.startsWith('.cargo/') || f.startsWith('modules/')) {
      rustAll = out.rustProto = web = webE2eAll = true
      continue
    }
    const crate = /^crates\/([^/]+)\/./.exec(f)
    if (crate) {
      if (!members().some((p) => p.dir === `crates/${crate[1]}`)) return full(members()) // a crate cargo does not list
      changedDirs.add(`crates/${crate[1]}`)
      continue
    }
    if (f.startsWith('crates/')) return full(members())

    if (f.startsWith('web/')) {
      web = true
      webFiles.push(f)
      if (f.startsWith('web/src/lib/engine/proto/crate/')) out.rustProto = true
      // Read by the helper's updater tests (and, for vercel.json, by the release script tests).
      if (f === 'web/vercel.json' || f === 'web/src/lib/i18n/helper.ts') helperInputs = true
      if (f === 'web/vercel.json') out.agentScripts = true
      // Adding, renaming or deleting a spec: the table's own tests must run (every spec placed in the table).
      if (f.startsWith('web/tests/e2e/')) out.agentScripts = true
      continue
    }
    if (f.startsWith('scripts/') || f.startsWith('.claude/')) { out.agentScripts = true; continue }
    return full(members()) // unknown: never skip on a guess
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
    if (WEB_CRATES.some((n) => built.has(n))) web = webE2eAll = true
  }
  out.web = web
  if (webE2eAll || webFiles.length) {
    // The table decides per web file; anything it cannot place, or specs it cannot read, is the whole suite.
    let existing = []
    try {
      existing = specs?.() ?? []
    } catch {
      /* unreadable: the whole suite */
    }
    const plan = planE2e(webFiles, existing, { all: webE2eAll })
    if (plan.run) {
      out.e2e = 'dev'
      out.e2eAll = plan.all
      out.e2eSpecs = plan.all ? [] : plan.specs
      out.e2eSmoke = plan.smoke
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
    e2e_all: String(r.e2eAll),
    e2e_specs: r.e2eSpecs.map((n) => `tests/e2e/${n}.spec.ts`).join(' '), // names are [a-z0-9-]: safe to split
    e2e_smoke: r.e2eSmoke.map((n) => `tests/e2e/${n}.spec.ts`).join(' '),
    e2e_smoke_grep: r.e2eGrep,
    e2e_shards: JSON.stringify(Array.from({ length: Math.max(r.e2eShards, 1) }, (_, i) => i + 1)), // the matrix; the job's `if` guards the empty case
    e2e_shard_total: String(Math.max(r.e2eShards, 1)),
    agent_scripts: String(r.agentScripts),
    rust: String(r.rustChecks || r.rustTest !== '' || r.rustProto),
  }
}
