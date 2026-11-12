// The spec files of one CI machine (#848): reads the tests Playwright lists for the run (`playwright test … --list
// --reporter=json`, so the configuration's projects and filters decide what exists) and the measures of
// scripts/ci/e2e-durations.json, deals the files out with `planShards` (e2e-plan.mjs) and writes this machine's
// lists to $GITHUB_OUTPUT. Every machine computes the same plan from the same checkout; the first one prints it whole
// to the run's summary.
//
//   node scripts/ci/e2e-shard.mjs --listing <listing.json> --shard <i>/<n> [--trimmed "<spec paths>"] [--core]
//
// --trimmed: the spec files an area run starts with --grep-invert @full (`e2e_specs`), costed without their @full
// tests; every other listed file runs whole (`e2e_specs_changed`, or the whole suite). --core: the core run goes on
// the first machine (CORE_SECONDS). Outputs, for each project P of the listing: `P_trimmed` and `P_whole` (spec paths
// relative to web/, space separated, possibly empty), and `projects`, the projects this machine runs.
import { appendFileSync, readFileSync } from 'node:fs'
import { CORE_SECONDS, DURATIONS_FILE, checkDurations, costUnits, planShards, reportTests, specFile } from './e2e-plan.mjs'

const args = process.argv.slice(2)
const opt = (name) => {
  const i = args.indexOf(name)
  return i === -1 ? undefined : args[i + 1]
}
const fail = (message) => {
  console.error(`e2e-shard: ${message}`)
  process.exit(1)
}

const shard = /^(\d+)\/(\d+)$/.exec(opt('--shard') ?? '')
if (!shard) fail('--shard <i>/<n> is required')
const [index, total] = [Number(shard[1]), Number(shard[2])]
if (index < 1 || index > total) fail(`shard ${index} of ${total}`)
const listingPath = opt('--listing')
if (!listingPath) fail('--listing <file> is required')

const tests = reportTests(JSON.parse(readFileSync(listingPath, 'utf8')))
// A listing with no test would leave every machine idle while the run reports green: never.
if (!tests.length) fail(`${listingPath} lists no test`)
let durations = {}
try {
  durations = checkDurations(JSON.parse(readFileSync(new URL(`../../${DURATIONS_FILE}`, import.meta.url), 'utf8')))
} catch (e) {
  // Unreadable measures change which machine runs a file, never whether it runs: every test then costs the default.
  console.error(`e2e-shard: ${DURATIONS_FILE} unreadable (${e.message}); every test costs the default`)
  durations = {}
}
const trimmed = new Set(
  (opt('--trimmed') ?? '')
    .split(/\s+/)
    .filter(Boolean)
    .map((p) => /([a-z0-9-]+)\.spec\.ts$/.exec(p)?.[1] ?? fail(`not a spec path: ${p}`)),
)
const plan = planShards(costUnits(tests, durations, trimmed), total, { preload: args.includes('--core') ? [CORE_SECONDS] : [] })

const projects = [...new Set(tests.map((t) => t.project))].sort()
const mine = plan[index - 1]
const outputs = { projects: projects.filter((p) => mine.units.some((u) => u.project === p)).join(' ') }
for (const p of projects) {
  for (const kind of ['trimmed', 'whole']) {
    outputs[`${p}_${kind}`] = mine.units
      .filter((u) => u.project === p && trimmed.has(u.spec) === (kind === 'trimmed'))
      .map((u) => specFile(u.spec))
      .join(' ')
  }
}

const lines = plan.map((m, i) => `machine ${i + 1}: ${m.seconds} s planned, ${m.units.map((u) => `${u.project}/${u.spec}`).join(', ') || 'nothing'}`)
console.log(lines.join('\n'))
console.log(outputs)
if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(outputs).map(([k, v]) => `${k}=${v}\n`).join(''))
if (process.env.GITHUB_STEP_SUMMARY && index === 1) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `e2e plan (seconds of tests and server starts per machine):\n\n${lines.map((l) => `- ${l}`).join('\n')}\n`)
