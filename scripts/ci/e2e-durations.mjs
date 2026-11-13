// Rebuilds scripts/ci/e2e-durations.json, the measured seconds of each end-to-end test per Playwright project that
// `planShards` (e2e-plan.mjs) deals the CI machines out with (#848), from the JSON reports of a CI run.
//
//   gh run download <run id> --repo sprkl/faulk --pattern 'e2e-report-*' --dir <folder>
//   node scripts/ci/e2e-durations.mjs <folder or report.json>... [--replace] [--table <file>]
//
// The reports are those every e2e machine uploads (`e2e-report-<shard>`, one file per Playwright command). Each
// measured test replaces its old measure; a test the reports do not hold keeps its old one, unless --replace (a
// whole-suite run of both projects: start from the reports alone). A spec no longer on disk loses its measures.
// A test skipped at run time is measured too (its cost is the few milliseconds it took). --table: another measures file
// than scripts/ci/e2e-durations.json (tests). Prints what changed per project.
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { DURATIONS_FILE, checkDurations, reportTests, specNames } from './e2e-plan.mjs'

const args = process.argv.slice(2)
const tableAt = args.indexOf('--table')
const paths = args.filter((a, i) => !a.startsWith('--') && !(tableAt !== -1 && i === tableAt + 1))
if (!paths.length) {
  console.error('usage: node scripts/ci/e2e-durations.mjs <folder or report.json>... [--replace] [--table <file>]')
  process.exit(1)
}
const files = (p) => (statSync(p).isDirectory() ? readdirSync(p).flatMap((e) => files(join(p, e))) : p.endsWith('.json') ? [p] : [])
const reports = paths.flatMap(files)
if (!reports.length) {
  console.error(`no .json report under ${paths.join(', ')}`)
  process.exit(1)
}

const target = tableAt === -1 ? new URL(`../../${DURATIONS_FILE}`, import.meta.url) : args[tableAt + 1]
const old = checkDurations(JSON.parse(readFileSync(target, 'utf8')))
const onDisk = new Set(specNames(readdirSync(new URL('../../web/tests/e2e/', import.meta.url))))
const measured = {}
for (const report of reports) {
  for (const t of reportTests(JSON.parse(readFileSync(report, 'utf8')))) {
    if (t.seconds === undefined) continue // a listing: no run
    ;(measured[t.project] ??= {})[t.key] = Math.round(t.seconds * 10) / 10
  }
}
if (!Object.keys(measured).length) {
  console.error('the reports hold no measured test')
  process.exit(1)
}

const next = { '//': old['//'] }
for (const project of [...new Set([...Object.keys(old), ...Object.keys(measured)])].filter((p) => !p.startsWith('//')).sort()) {
  const merged = { ...(args.includes('--replace') ? {} : old[project]), ...measured[project] }
  const kept = Object.entries(merged).filter(([key]) => onDisk.has(key.split('.spec.ts')[0]))
  if (!kept.length) continue
  next[project] = Object.fromEntries(kept.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
  const before = Object.values(old[project] ?? {}).reduce((a, s) => a + s, 0)
  const after = Object.values(next[project]).reduce((a, s) => a + s, 0)
  console.log(`${project}: ${Object.keys(measured[project] ?? {}).length} tests measured, ${kept.length} kept; ${Math.round(before)} s -> ${Math.round(after)} s in all`)
}
checkDurations(next)
// One test per line, so that a refresh reads as a diff of tests.
const body = Object.entries(next)
  .map(([k, v]) => (k.startsWith('//') ? `  ${JSON.stringify(k)}: ${JSON.stringify(v)}` : `  ${JSON.stringify(k)}: {\n${Object.entries(v).map(([t, s]) => `    ${JSON.stringify(t)}: ${s}`).join(',\n')}\n  }`))
  .join(',\n')
writeFileSync(target, `{\n${body}\n}\n`)
console.log(`wrote ${tableAt === -1 ? DURATIONS_FILE : target}`)
