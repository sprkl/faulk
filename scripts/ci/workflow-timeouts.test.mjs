// Every job of every workflow has a `timeout-minutes` (#447): a hung job once used 6 h of runner time (the default).
import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { test } from 'node:test'

const dir = new URL('../../.github/workflows/', import.meta.url)

/** `{job: timeout | null}` of a workflow, read line by line (jobs are the 2-space keys under `jobs:`, no YAML dependency). */
function jobTimeouts(text) {
  const jobs = {}
  let inJobs = false
  let job = null
  for (const line of text.split(/\r?\n/)) {
    if (/^jobs:\s*$/.test(line)) inJobs = true
    else if (/^\S/.test(line)) inJobs = false
    if (!inJobs) continue
    const m = /^ {2}([A-Za-z0-9_-]+):\s*$/.exec(line)
    if (m) {
      job = m[1]
      jobs[job] = null
    }
    const t = /^ {4}timeout-minutes:\s*(\d+)\s*(#.*)?$/.exec(line)
    if (t && job) jobs[job] = Number(t[1])
  }
  return jobs
}

test('the reader finds jobs with and without a timeout', () => {
  const yml = 'name: x\non:\n  push:\njobs:\n  a:\n    runs-on: u\n    timeout-minutes: 5\n  b:\n    runs-on: u\n'
  assert.deepEqual(jobTimeouts(yml), { a: 5, b: null })
})

test('every job of every workflow has a timeout of 1 to 120 minutes', () => {
  const files = readdirSync(dir).filter((f) => /\.ya?ml$/.test(f))
  assert.ok(files.length >= 4)
  let count = 0
  for (const f of files) {
    const jobs = jobTimeouts(readFileSync(new URL(f, dir), 'utf8'))
    assert.ok(Object.keys(jobs).length > 0, `${f}: no job found`)
    for (const [job, t] of Object.entries(jobs)) {
      count++
      assert.ok(Number.isInteger(t) && t >= 1 && t <= 120, `${f}: job "${job}" has no sane timeout-minutes (${t})`)
    }
  }
  assert.ok(count >= 12, `only ${count} jobs found`)
})

test('a step-level or commented timeout does not count for the job', () => {
  const yml = `jobs:
  a:
    runs-on: u
    steps:
      - run: x
        timeout-minutes: 5
  b:
    runs-on: u
    # timeout-minutes: 5
`
  assert.deepEqual(jobTimeouts(yml), { a: null, b: null })
})
