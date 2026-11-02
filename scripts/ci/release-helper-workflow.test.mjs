import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

// The macOS leg of release-helper.yml runs for a production release only (a version), never for
// a trial build (pull request, dispatch without a version): macOS minutes are expensive (#452).
const text = readFileSync(new URL('../../.github/workflows/release-helper.yml', import.meta.url), 'utf8')

const m = text.match(/include: \$\{\{ fromJSON\((.+?) != '' && '(\[.*?\])' \|\| '(\[.*?\])'\) \}\}/)

test('the unix matrix is chosen by whether the run has a version', () => {
  assert.ok(m, 'matrix include is a fromJSON(version != "" && release || trial) expression')
  assert.equal(m[1], 'needs.version.outputs.value')
})

test('a trial build has no macOS leg, only Linux', () => {
  const trial = JSON.parse(m[3])
  assert.deepEqual(trial.map((l) => l.channel), ['linux'])
  assert.ok(!trial.some((l) => /macos|osx/.test(JSON.stringify(l))))
})

test('a production release keeps the macOS and Linux legs', () => {
  const release = JSON.parse(m[2])
  assert.deepEqual(release.map((l) => l.channel), ['osx', 'linux'])
  assert.equal(release[0].os, 'macos-latest')
  assert.equal(release[0].runtime, 'osx-arm64')
})

test('no other line of the workflow runs on macOS', () => {
  const lines = text
    .split(/\r?\n/)
    .filter((l) => /macos/.test(l) && !l.trim().startsWith('#'))
  // Only the matrix line itself, whose literals the tests above parse.
  assert.equal(lines.length, 1)
  assert.match(lines[0], /^\s*include: \$\{\{ fromJSON\(/)
})

test('a trial build has no version: only a tag or a dispatch with a version sets one', () => {
  assert.match(text, /refs\/tags\/helper-v\*\) value="\$\{REF#refs\/tags\/helper-v\}"/)
  assert.match(text, /if \[ "\$EVENT" = workflow_dispatch \]; then value="\$\{INPUT#v\}"; else value=""; fi/)
})

test('the trial dry run publishes the Windows output only, so it needs no osx artifact', () => {
  assert.match(text, /publish\.mjs publish --dir out --os win .*--dry-run/)
})
