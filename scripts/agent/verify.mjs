// One command to check a change before the pull request (issue #372; docs/agents/verification.md):
// Targeted by default (#447): the Rust crates the diff touches and their dependents (cargo test -p), the WASM builds and parity tests only when
// the engine or what it embeds changes, fmt and clippy of the workspace, the agent scripts' tests, and the web part: all chosen by the CI selection
// (scripts/ci/select-core.mjs). --full (alias --all) runs everything; so does an unreadable base or workspace.
//   node scripts/agent/verify.mjs [--base origin/develop] [--full|--all] [--no-e2e] [--dry-run]
//                                 [--ports dev,preview,helper] [--keep-going] [--verbose] [--clean] [--prune-cache] [--fix]
// It runs, from the web app: formatting (Prettier, as the CI does), typecheck, lint, vitest, then the end-to-end tier of the CI decision (#533,
// scripts/ci/e2e-plan.mjs): the area specs plus the core tests, the core tests alone for a shared page file, the whole suite for a contract
// or a configuration; the summary names the tier and why. It picks free
// ports for the servers, so two checkouts never collide, and ends with a short summary. --fix rewrites the formatting (pnpm format) and
// applies the auto-fixable lint (eslint --fix) instead of only checking them, then goes on. Works in Git
// Bash, PowerShell and on Linux. Exit code 0 when everything passed.
//
// Build cache: cargo builds into one directory shared by every checkout (CARGO_TARGET_DIR, default
// %LOCALAPPDATA%\faulk-build\target, or $XDG_CACHE_HOME/faulk-build/target), emptied by --prune-cache
// once it passes FAULK_BUILD_CACHE_MAX_GB (default 8; the script only warns on its own). FAULK_SHARED_TARGET=0 keeps the checkout's own target/.
import { execFileSync, spawn } from 'node:child_process'
import { existsSync, readFileSync, readdirSync, rmSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { FULL_TAG, TABLE_FILE } from '../ci/e2e-plan.mjs'
import { e2eSummary, packagesFromMetadata } from '../ci/select-core.mjs'
import { dirSizeUpTo, freePorts, parsePorts, portIsFree, sharedTargetDir, specFile, specNames, summary } from './verify-core.mjs'
import { evidenceLine, planVerify, rootSteps } from './verify-plan.mjs'

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..')
const web = join(root, 'web')
const args = process.argv.slice(2)
const flag = (name) => args.includes(name)
const opt = (name) => {
  const i = args.indexOf(name)
  return i === -1 ? undefined : args[i + 1]
}

const git = (...a) => execFileSync('git', a, { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).split('\n').map((s) => s.trim()).filter(Boolean)
const gitOk = (...a) => {
  try {
    execFileSync('git', a, { cwd: root, stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

function resolveBase() {
  const wanted = opt('--base')
  if (wanted) return wanted
  return gitOk('rev-parse', '--verify', '-q', 'origin/develop') ? 'origin/develop' : 'develop'
}

/**
 * Committed, staged, unstaged and untracked files, against the point where this branch left the base.
 * Null when the base cannot be read (never fetched, no common ancestor): the caller then runs everything.
 */
function changedFiles(base) {
  let mergeBase
  try {
    mergeBase = git('merge-base', base, 'HEAD')[0]
  } catch {
    return null
  }
  if (!mergeBase) return null
  const tracked = git('diff', '--name-only', '--no-renames', mergeBase)
  const untracked = git('ls-files', '--others', '--exclude-standard')
  return [...new Set([...tracked, ...untracked])]
}

/** The e2e table file at the merge base and in the working tree (#533: an edit of the table alone; throws when unreadable: everything runs). */
function tableTexts(base) {
  const mergeBase = git('merge-base', base, 'HEAD')[0]
  const before = execFileSync('git', ['show', `${mergeBase}:${TABLE_FILE}`], { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 })
  const files = [...git('ls-files'), ...git('ls-files', '--others', '--exclude-standard')]
  return { base: before, head: readFileSync(join(root, TABLE_FILE), 'utf8'), files }
}

/** How many tests of `specs` carry the @full tag (a `--list` of Playwright); null when it cannot be counted. */
function countFull(specs, env) {
  try {
    const out = execFileSync([`pnpm exec playwright test`, ...specs.map(specFile), '--list', '--grep', FULL_TAG].join(' '), { cwd: web, env, encoding: 'utf8', shell: true, stdio: ['ignore', 'pipe', 'pipe'] })
    const m = /Total: (\d+) tests?/.exec(out)
    return m ? Number(m[1]) : 0
  } catch (e) {
    // No test matches: Playwright says so and exits non-zero.
    return /No tests found/i.test(`${e.stdout ?? ''}${e.stderr ?? ''}`) ? 0 : null
  }
}

/** The workspace crates, from `cargo metadata` (throws when cargo cannot read it: the plan then runs everything). */
function workspacePackages() {
  const json = execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
  return packagesFromMetadata(JSON.parse(json))
}

/** Points cargo at the shared build cache. Returns a warning for the summary when the cache is over its limit. */
function setupBuildCache(env) {
  env.CARGO_INCREMENTAL ??= '0'
  if (env.CARGO_TARGET_DIR || process.env.FAULK_SHARED_TARGET === '0') {
    if (flag('--prune-cache')) console.log('--prune-cache ignored: CARGO_TARGET_DIR is yours, or FAULK_SHARED_TARGET=0 (the shared build cache is not in use)')
    return ''
  }
  const dir = sharedTargetDir(process.env, process.platform, homedir())
  if (!dir) return ''
  env.CARGO_TARGET_DIR = dir
  // Emptied only on request: another checkout's cargo may be writing into it right now.
  if (flag('--prune-cache')) {
    rmSync(dir, { recursive: true, force: true })
    console.log(`build cache emptied: ${dir}`)
    return ''
  }
  const limit = (Number(process.env.FAULK_BUILD_CACHE_MAX_GB) || 8) * 1024 ** 3
  if (existsSync(dir) && dirSizeUpTo(dir, limit) > limit) {
    return `WARNING: the build cache is above ${limit / 1024 ** 3} GB (${dir}). When no other build is running: node scripts/agent/verify.mjs --prune-cache`
  }
  return ''
}

/** Child processes still running: stopped with the script (on Windows a shell does not pass the signal on). */
const running = new Set()
function stopRunning() {
  for (const proc of running) {
    try {
      if (process.platform === 'win32') execFileSync('taskkill', ['/pid', String(proc.pid), '/T', '/F'], { stdio: 'ignore' })
      else process.kill(-proc.pid, 'SIGTERM') // the whole group: pnpm, playwright, the dev server, Chromium
    } catch {
      /* already gone */
    }
  }
}
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    stopRunning()
    process.exit(130)
  })
}
process.on('exit', stopRunning)

/** Runs a command in the web app; the output is kept and shown (tail) only when it fails, or with --verbose. */
function run(name, command, commandArgs, env, cwd = web) {
  return new Promise((resolve) => {
    const started = Date.now()
    const verbose = flag('--verbose')
    console.log(`> ${name}`)
    // Windows resolves `pnpm` (a .cmd shim) only through a shell; the arguments are ours, never user text.
    const child =
      process.platform === 'win32'
        ? spawn([command, ...commandArgs].join(' '), { cwd, env, shell: true, stdio: ['ignore', 'pipe', 'pipe'] })
        : spawn(command, commandArgs, { cwd, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] }) // own process group
    running.add(child)
    let log = ''
    const onData = (d) => {
      if (verbose) process.stdout.write(d)
      else log = (log + d).slice(-20000)
    }
    child.stdout.on('data', onData)
    child.stderr.on('data', onData)
    child.on('error', (e) => {
      log += String(e)
    })
    child.on('close', (code) => {
      running.delete(child)
      const ok = code === 0
      if (!ok && !verbose) console.log(log.split('\n').slice(-40).join('\n'))
      resolve({ name, ok, ms: Date.now() - started })
    })
  })
}

async function main() {
  const started = Date.now()
  const forcedPorts = flag('--ports') ? parsePorts(opt('--ports')) : null // a typo fails before anything runs
  const base = resolveBase()
  const listed = changedFiles(base)
  if (!listed) console.log(`cannot compare with ${base} (run "git fetch origin develop"): running the whole suite`)
  const files = listed ?? []
  const existing = specNames(readdirSync(join(web, 'tests', 'e2e')))
  // The CI selection itself (#447, scripts/ci/select-core.mjs): crates touched and their dependents, WASM builds and
  // parity only when the page or the engine embeds the change, and the e2e plan below. --full (or --all): everything.
  const plan = planVerify({ files: listed, full: flag('--full') || flag('--all'), packages: workspacePackages, specs: () => existing, table: () => tableTexts(base) })
  const decision = plan.decision
  // The end-to-end plan is the decision's own (#533): the one CI computes for the same files, tier and reasons included.
  const wantWeb = plan.mode === 'full' || decision.web
  const e2eAll = decision.e2eAll
  if (e2eAll && !existing.length) throw new Error('no end-to-end spec found in web/tests/e2e')
  const noE2e = flag('--no-e2e') || !wantWeb || decision.e2e === ''
  const specs = noE2e ? [] : e2eAll ? existing : decision.e2eSpecs
  const changed = noE2e || e2eAll ? [] : decision.e2eChanged
  const core = noE2e ? [] : decision.e2eCore
  const tierLine = noE2e && decision.e2e !== '' ? `${e2eSummary(decision)} [skipped: --no-e2e]` : e2eSummary(decision)

  if (flag('--dry-run')) {
    console.log(`mode ${plan.mode} (${plan.why})\nrust: ${rootSteps(decision).map((s) => s.name).join(', ') || 'none'}\n${evidenceLine({ mode: plan.mode, base, decision })}`)
    console.log(`base ${base}\nfiles:\n${files.map((f) => `  ${f}`).join('\n')}\nweb checks: ${wantWeb}\n${tierLine}\nspecs (${specs.length}${e2eAll ? ', whole suite' : `, without the ${FULL_TAG} tests`}): ${specs.join(', ')}\nchanged specs, run whole (${changed.length}): ${changed.join(', ')}\ncore tests only (${core.length}): ${core.join(', ')}`)
    if (decision.e2eGrep) console.log(`core grep: ${decision.e2eGrep}`)
    return 0
  }

  const env = { ...process.env }
  const cacheWarning = setupBuildCache(env)
  const steps = []
  let excluded // how many @full tests the area run leaves out (counted with `--list`)
  const go = async (name, command, commandArgs, extraEnv = {}) => {
    const step = await run(name, command, commandArgs, { ...env, ...extraEnv })
    steps.push(step)
    return step.ok || flag('--keep-going')
  }

  let proceed = true
  // Rust and the other root steps first (cheap failures first); the agent scripts step may fail on Windows for
  // reasons that predate the change: the summary names the step, --keep-going runs the rest.
  for (const s of rootSteps(decision)) {
    if (!proceed) break
    const glob = process.platform === 'win32' ? s.args.map((a) => (a.includes('*') ? `"${a}"` : a)) : s.args
    const step = await run(s.name, s.command, glob, env, s.cwd ? join(root, s.cwd) : root)
    steps.push(step)
    proceed = step.ok || flag('--keep-going')
  }
  if (!proceed) {
    // a root step failed: the web steps are not run
  } else if (!wantWeb) {
    steps.push({ name: 'web checks', ok: true, skipped: true, ms: 0, note: 'no change the page is built from' })
  } else {
    if (!existsSync(join(web, 'node_modules'))) proceed = await go('install', 'pnpm', ['install', '--frozen-lockfile'])
    // --fix: the auto-fixable lint first, then the formatter, which has the last word on layout (as in the PostToolUse hook).
    if (proceed && flag('--fix')) proceed = await go('lint (fix)', 'pnpm', ['lint', '--fix'])
    if (proceed) proceed = flag('--fix') ? await go('format (fix)', 'pnpm', ['format']) : await go('format', 'pnpm', ['format:check'])
    // The CI `checks` job also runs gen:check: the generated protocol files match the Rust crates.
    if (proceed && !flag('--fix')) proceed = await go('gen:check', 'pnpm', ['gen:check'])
    if (proceed) proceed = await go('typecheck', 'pnpm', ['typecheck'])
    if (proceed && !flag('--fix')) proceed = await go('lint', 'pnpm', ['lint'])
    // `pnpm test` also builds the two WASM modules the dev server serves.
    if (proceed) proceed = await go('vitest', 'pnpm', ['test'])
    if (proceed && (specs.length || changed.length || core.length)) {
      const ports = forcedPorts ?? (await freePorts(3))
      const e2eEnv = {
        ...env,
        FAULK_E2E_PROJECTS: 'dev',
        FAULK_E2E_DEV_PORT: String(ports[0]),
        FAULK_E2E_PREVIEW_PORT: String(ports[1]),
        FAULK_E2E_HELPER_PORT: String(ports[2]),
      }
      const playwright = ['exec', 'playwright', 'test']
      // CI runs these commands too (.github/workflows/ci.yml): the area specs without the @full tests, the
      // specs whose own file changed whole, then the core tests of the others (never a @full one).
      const runs = []
      const invert = ['--grep-invert', FULL_TAG, '--pass-with-no-tests']
      if (!e2eAll && specs.length) excluded = countFull(specs, e2eEnv)
      if (specs.length) runs.push(await run(`e2e (${specs.length})`, 'pnpm', [...playwright, ...(e2eAll ? [] : [...specs.map(specFile), ...invert])], e2eEnv))
      if (changed.length && (runs.every((r) => r.ok) || flag('--keep-going'))) runs.push(await run(`changed (${changed.length})`, 'pnpm', [...playwright, ...changed.map(specFile)], e2eEnv))
      if (core.length && (runs.every((r) => r.ok) || flag('--keep-going'))) {
        // The pattern holds `|`, `^` and `.*`: quoted for the shell the Windows branch of run() goes through.
        const grep = process.platform === 'win32' ? `"${decision.e2eGrep}"` : decision.e2eGrep
        runs.push(await run(`core (${core.length})`, 'pnpm', [...playwright, ...core.map(specFile), '--grep', grep, '--grep-invert', FULL_TAG], e2eEnv))
      }
      const busy = []
      for (const p of ports) if (!(await portIsFree(p))) busy.push(p)
      runs[0].note = `ports ${ports.join('/')}${busy.length ? `, still busy after the run: ${busy.join(', ')}` : ''}`
      steps.push(...runs)
    }
  }

  const fullNote = excluded === undefined ? '' : excluded === null ? `; ${FULL_TAG} tests excluded: not counted` : `; ${excluded} ${FULL_TAG} test(s) of the area specs left to the whole suite`
  console.log(`\n${summary(steps, { base, files, specs, changed, core, all: e2eAll, tier: `${tierLine}${fullNote}` })}`)
  console.log(`mode: ${plan.mode} (${plan.why})\nfor record.mjs --rust: ${evidenceLine({ mode: plan.mode, base, decision })}`)
  if (cacheWarning) console.log(cacheWarning)
  console.log(`total ${((Date.now() - started) / 1000).toFixed(0)} s wall`)
  if (flag('--clean')) {
    for (const d of ['test-results', 'playwright-report', '.svelte-kit']) rmSync(join(web, d), { recursive: true, force: true })
    console.log('cleaned: web/test-results, web/playwright-report, web/.svelte-kit (node_modules and the build cache stay)')
  }
  return steps.every((s) => s.ok || s.skipped) ? 0 : 1
}

try {
  process.exitCode = await main()
} catch (e) {
  console.error(`verify: ${e.message}`)
  process.exitCode = 2
}
