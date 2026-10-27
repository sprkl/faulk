// CI change selection (issue #330). In a workflow: reads the pull request's changed files from
// the checkout and writes the decisions to $GITHUB_OUTPUT. By hand (dry run, no checkout needed):
//   node scripts/ci/select.mjs --event pull_request --base develop --files web/src/a.ts crates/engine/src/lib.rs
//   node scripts/ci/select.mjs --event pull_request --base main --files docs/x.md
// Options: --json prints the full decision, --files-from <file|-> reads one path per line.
import { execFileSync } from 'node:child_process'
import { appendFileSync, readFileSync, readdirSync } from 'node:fs'
import { specNames } from './e2e-plan.mjs'
import { fullDecision, packagesFromMetadata, select, toOutputs } from './select-core.mjs'

const args = process.argv.slice(2)
const opt = (name) => {
  const i = args.indexOf(name)
  return i === -1 ? undefined : args[i + 1]
}
const event = opt('--event') ?? process.env.GITHUB_EVENT_NAME ?? ''
const base = opt('--base') ?? process.env.GITHUB_BASE_REF ?? ''

function listFiles() {
  try {
    const from = opt('--files-from')
    if (from) return readFileSync(from === '-' ? 0 : from, 'utf8').split('\n').filter(Boolean)
    const i = args.indexOf('--files')
    if (i !== -1) return args.slice(i + 1).filter((a) => !a.startsWith('--'))
    if (event !== 'pull_request') return []
    // The checkout of a pull request is its merge commit: its first parent is the base tip.
    const out = execFileSync('git', ['diff', '--name-only', '--no-renames', 'HEAD^1', 'HEAD'], { encoding: 'utf8' })
    return out.split('\n').filter(Boolean)
  } catch (e) {
    console.error(`cannot list the changed files (${e.message}): running everything`)
    return null
  }
}

function workspacePackages() {
  {
    const json = execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], {
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    })
    return packagesFromMetadata(JSON.parse(json))
  }
}

const files = listFiles()
if (files) console.log(files.join('\n'))
let result
try {
  // The spec names on disk (an unreadable directory counts as no specs: the whole suite).
  const specs = () => specNames(readdirSync(new URL('../../web/tests/e2e/', import.meta.url)))
  result = select({ event, base, files, packages: workspacePackages, specs })
} catch (e) {
  // An unreadable workspace must never skip a test.
  console.error(`cannot read the cargo workspace (${e.message}): running everything`)
  result = fullDecision()
}
const outputs = toOutputs(result)
console.log(args.includes('--json') ? JSON.stringify(result, null, 2) : outputs)
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(outputs).map(([k, v]) => `${k}=${v}\n`).join(''))
}
