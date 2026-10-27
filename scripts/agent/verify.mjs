// One command to check a change before the pull request (issue #372; docs/agents/verification.md):
//   node scripts/agent/verify.mjs [--base origin/develop] [--all] [--no-e2e] [--dry-run]
//                                 [--ports dev,preview,helper] [--keep-going] [--verbose] [--clean] [--prune-cache]
// It runs, from the web app: typecheck, lint, vitest, then the end-to-end specs the diff against the
// base touches plus the smoke tests (one table with the CI, scripts/ci/e2e-plan.mjs; the whole suite when a shared file changed). It picks free
// ports for the servers, so two checkouts never collide, and ends with a short summary. Works in Git
// Bash, PowerShell and on Linux. Exit code 0 when everything passed.
//
// Build cache: cargo builds into one directory shared by every checkout (CARGO_TARGET_DIR, default
// %LOCALAPPDATA%\faulk-build\target, or $XDG_CACHE_HOME/faulk-build/target), emptied by --prune-cache
// once it passes FAULK_BUILD_CACHE_MAX_GB (default 8; the script only warns on its own). FAULK_SHARED_TARGET=0 keeps the checkout's own target/.
import { execFileSync, spawn } from 'node:child_process'
import { existsSync, readdirSync, rmSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { dirSizeUpTo, freePorts, needsWeb, parsePorts, planE2e, portIsFree, sharedTargetDir, specFile, specNames, summary } from './verify-core.mjs'

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
function run(name, command, commandArgs, env) {
  return new Promise((resolve) => {
    const started = Date.now()
    const verbose = flag('--verbose')
    console.log(`> ${name}`)
    // Windows resolves `pnpm` (a .cmd shim) only through a shell; the arguments are ours, never user text.
    const child =
      process.platform === 'win32'
        ? spawn([command, ...commandArgs].join(' '), { cwd: web, env, shell: true, stdio: ['ignore', 'pipe', 'pipe'] })
        : spawn(command, commandArgs, { cwd: web, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] }) // own process group
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
  // The same plan as the CI (scripts/ci/e2e-plan.mjs): whole specs, plus the smoke tests of the others.
  const selection = planE2e(files, existing, { all: flag('--all') || !listed })
  const wantWeb = flag('--all') || !listed || needsWeb(files)
  if (selection.run && selection.all && !selection.specs.length) throw new Error('no end-to-end spec found in web/tests/e2e')
  const noE2e = flag('--no-e2e') || !wantWeb || !selection.run
  const specs = noE2e ? [] : selection.specs
  const smoke = noE2e ? [] : selection.smoke

  if (flag('--dry-run')) {
    console.log(`base ${base}\nfiles:\n${files.map((f) => `  ${f}`).join('\n')}\nweb checks: ${wantWeb}\nspecs (${specs.length}${selection.all ? ', whole suite' : ''}): ${specs.join(', ')}\nsmoke only (${smoke.length}): ${smoke.join(', ')}`)
    for (const [why, list] of Object.entries(selection.reasons)) console.log(`  ${why}: ${list.join(', ')}`)
    return 0
  }

  const env = { ...process.env }
  const cacheWarning = setupBuildCache(env)
  const steps = []
  const go = async (name, command, commandArgs, extraEnv = {}) => {
    const step = await run(name, command, commandArgs, { ...env, ...extraEnv })
    steps.push(step)
    return step.ok || flag('--keep-going')
  }

  let proceed = true
  if (!wantWeb) {
    steps.push({ name: 'web checks', ok: true, skipped: true, ms: 0, note: 'no change under web/, crates/ or the build inputs' })
  } else {
    if (!existsSync(join(web, 'node_modules'))) proceed = await go('install', 'pnpm', ['install', '--frozen-lockfile'])
    if (proceed) proceed = await go('typecheck', 'pnpm', ['typecheck'])
    if (proceed) proceed = await go('lint', 'pnpm', ['lint'])
    // `pnpm test` also builds the two WASM modules the dev server serves.
    if (proceed) proceed = await go('vitest', 'pnpm', ['test'])
    if (proceed && (specs.length || smoke.length)) {
      const ports = forcedPorts ?? (await freePorts(3))
      const e2eEnv = {
        ...env,
        FAULK_E2E_PROJECTS: 'dev',
        FAULK_E2E_DEV_PORT: String(ports[0]),
        FAULK_E2E_PREVIEW_PORT: String(ports[1]),
        FAULK_E2E_HELPER_PORT: String(ports[2]),
      }
      const playwright = ['exec', 'playwright', 'test']
      // CI runs these two commands too (.github/workflows/ci.yml): the whole specs, then the smoke tests of the others.
      const runs = []
      if (specs.length) runs.push(await run(`e2e (${specs.length})`, 'pnpm', [...playwright, ...specs.map(specFile)], e2eEnv))
      if (smoke.length && (runs.every((r) => r.ok) || flag('--keep-going'))) {
        // The pattern holds `|` and `.*`: quoted for the shell the Windows branch of run() goes through.
        const grep = process.platform === 'win32' ? `"${selection.grep}"` : selection.grep
        runs.push(await run(`smoke (${smoke.length})`, 'pnpm', [...playwright, ...smoke.map(specFile), '--grep', grep], e2eEnv))
      }
      const busy = []
      for (const p of ports) if (!(await portIsFree(p))) busy.push(p)
      runs[0].note = `ports ${ports.join('/')}${busy.length ? `, still busy after the run: ${busy.join(', ')}` : ''}`
      steps.push(...runs)
    }
  }

  console.log(`\n${summary(steps, { base, files, specs, smoke, all: selection.all })}`)
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
