// What a local check runs (#447), pure: no process is started here. Tested by verify-plan.test.mjs.
// The selection is the CI one (`select` of scripts/ci/select-core.mjs, a pull request against `develop`), so
// the local run selects what the CI selects. Full (the whole workspace, the whole suite) on request, when the
// changed files are unknown, or when the workspace cannot be read: a guess never skips a test.
import { fullDecision, select } from '../ci/select-core.mjs'

/**
 * @param {{files: string[]|null, full?: boolean, packages: () => object[], specs: () => string[], table?: () => {base: string, head: string}}} input
 *   `files: null`: the base could not be read. `table`: the e2e table file at the base and now (select-core.mjs).
 * @returns {{mode: 'targeted'|'full', why: string, decision: ReturnType<typeof select>}}
 */
export function planVerify({ files, full = false, packages, specs, table }) {
  const fullPlan = (why) => {
    let ws = []
    try {
      ws = packages()
    } catch {
      /* the full decision needs no workspace */
    }
    // verify.mjs runs the `dev` project only (the production build is the release's): plan what it runs.
    return { mode: 'full', why, decision: fullDecision(ws, `verify.mjs full: ${why}`, 'dev') }
  }
  if (full) return fullPlan('requested')
  if (!Array.isArray(files)) return fullPlan('changed files unknown')
  try {
    const decision = select({ event: 'pull_request', base: 'develop', files, packages, specs, table })
    const isFull = decision.mode === 'full'
    return { mode: isFull ? 'full' : 'targeted', why: isFull ? 'a shared or unknown file changed' : 'changed files', decision }
  } catch (e) {
    return fullPlan(`workspace unreadable: ${e.message}`)
  }
}

const PROTO = 'web/src/lib/engine/proto/crate'

/** The commands run from the repository root that a decision asks for, in order (the web steps are separate). */
export function rootSteps(d) {
  const steps = []
  if (d.rustChecks) {
    steps.push({ name: 'fmt', command: 'cargo', args: ['fmt', '--all', '--check'] })
    steps.push({ name: 'clippy', command: 'cargo', args: ['clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'] })
  }
  if (d.rustTest) steps.push({ name: 'cargo test', command: 'cargo', args: ['test', ...d.rustTest.split(' ')] })
  if (d.rustWasmEngine) {
    steps.push({ name: 'wasm engine', command: 'cargo', args: ['build', '-p', 'engine-wasm', '--target', 'wasm32-wasip1-threads', '--release'] })
    steps.push({ name: 'wasm engine u', command: 'cargo', args: ['build', '-p', 'engine-wasm', '--target', 'wasm32-unknown-unknown', '--release'] })
  }
  if (d.rustWasmProject) steps.push({ name: 'wasm project', command: 'cargo', args: ['build', '-p', 'project-wasm', '--target', 'wasm32-unknown-unknown', '--release'] })
  if (d.rustProto) {
    steps.push({ name: 'proto fmt', command: 'cargo', args: ['fmt', '--check'], cwd: PROTO })
    for (const t of ['wasm32-unknown-unknown', 'wasm32-wasip1-threads']) {
      steps.push({ name: `proto clippy ${t.includes('wasip1') ? 'wasi' : 'web'}`, command: 'cargo', args: ['clippy', '--release', '--target', t, '--', '-D', 'warnings'], cwd: PROTO })
    }
  }
  if (d.agentScripts) steps.push({ name: 'agent scripts', command: 'node', args: ['--test', 'scripts/**/*.test.mjs'] })
  return steps
}

/** The text to give to `record.mjs verified --rust`: what the run covered. */
export function evidenceLine({ mode, base, decision }) {
  const wasm = [decision.rustWasmEngine && 'engine', decision.rustWasmProject && 'project'].filter(Boolean).join('+') || 'none'
  return `verify.mjs ${mode} vs ${base}: fmt+clippy ${decision.rustChecks ? 'workspace' : 'not needed'}, cargo test ${decision.rustTest || 'not needed'}, wasm builds ${wasm}`
}
