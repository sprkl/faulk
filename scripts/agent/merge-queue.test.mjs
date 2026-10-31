// Tests of the lead's merge script and of the regeneration tool (#426), on throwaway git repositories
// (a bare origin and a clone) with stub `gh` and `pnpm` programs first on PATH. The scripts are copied
// into the clone and run from there (they `cd` to their own repository), so nothing here reads or
// pushes to the real repository; a stub that meets a call it does not know fails with status 99.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { delimiter, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'

const tools = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '.claude', 'tools')
const GEN = 'web/src/lib/generated'

const GH = `#!/usr/bin/env bash
echo "gh $*" >> "$MQ_LOG"
case "$*" in
  *"--json mergeable,mergeStateStatus"*) echo "$MQ_STATE" ;;
  *"--json headRefName"*) echo "$MQ_HEAD" ;;
  *"--json headRefOid"*)
    n=$(cat "$MQ_LOG.n" 2>/dev/null || echo 0); echo $((n + 1)) > "$MQ_LOG.n"
    [ "$n" = 0 ] && echo old || echo new ;;
  "pr update-branch"*) ;;
  "run list"*) echo 1 ;;
  "pr checks"*"--watch"*) ;;
  "pr checks"*"--json bucket"*) echo pass ;;
  "pr merge"*) ;;
  *) exit 99 ;;
esac
`
const PNPM = `#!/usr/bin/env bash
echo "$(basename "$PWD")|$*" >> "$MQ_LOG"
[ "$1" = gen:protocol ] && printf 'regenerated\\n' > src/lib/generated/schema.json
exit 0
`

function git(cwd, ...args) {
  const r = spawnSync('git', args, { cwd, encoding: 'utf8' })
  assert.equal(r.status, 0, `git ${args.join(' ')}: ${r.stderr}`)
  return r.stdout.trim()
}

/** A bare origin, a clone with develop and a `pr` branch from the same base, and the stubs. */
function world() {
  const dir = mkdtempSync(join(tmpdir(), 'faulk-mq-'))
  const origin = join(dir, 'origin.git')
  const work = join(dir, 'work')
  const stubs = join(dir, 'stubs')
  mkdirSync(stubs)
  for (const [name, body] of [['gh', GH], ['pnpm', PNPM]]) {
    writeFileSync(join(stubs, name), body)
    chmodSync(join(stubs, name), 0o755)
  }
  git(dir, 'init', '-q', '--bare', '-b', 'develop', origin)
  git(dir, 'clone', '-q', origin, work)
  git(work, 'config', 'user.email', 't@example.com')
  git(work, 'config', 'user.name', 'Test')
  git(work, 'config', 'commit.gpgsign', 'false')
  mkdirSync(join(work, '.claude', 'tools'), { recursive: true })
  mkdirSync(join(work, GEN), { recursive: true })
  mkdirSync(join(work, 'web', 'src', 'other'), { recursive: true })
  for (const f of ['merge-queue.sh', 'regen-generated.sh']) copyFileSync(join(tools, f), join(work, '.claude', 'tools', f))
  writeFileSync(join(work, GEN, 'schema.json'), ['a', 'b', 'c', 'd', 'e'].join('\n') + '\n')
  writeFileSync(join(work, 'web', 'src', 'other', 'x.txt'), 'x\n')
  git(work, 'add', '-A')
  git(work, 'commit', '-q', '-m', 'base')
  git(work, 'push', '-q', 'origin', 'develop')
  git(work, 'checkout', '-q', '-b', 'pr')
  const log = join(dir, 'log.txt')
  writeFileSync(log, '')
  const env = {
    ...process.env,
    PATH: stubs + delimiter + process.env.PATH,
    MQ_LOG: log,
    MQ_STATE: 'MERGEABLE BEHIND',
    MQ_HEAD: 'pr',
    CARGO_TARGET_DIR: join(dir, 'target'),
    GIT_CONFIG_COUNT: '1',
    GIT_CONFIG_KEY_0: 'commit.gpgsign',
    GIT_CONFIG_VALUE_0: 'false',
    GIT_AUTHOR_NAME: 'Test',
    GIT_AUTHOR_EMAIL: 't@example.com',
    GIT_COMMITTER_NAME: 'Test',
    GIT_COMMITTER_EMAIL: 't@example.com',
  }
  const commit = (branch, file, text, push = true) => {
    git(work, 'checkout', '-q', branch)
    writeFileSync(join(work, file), text)
    git(work, 'add', '-A')
    git(work, 'commit', '-q', '-m', `${branch}: ${file}`)
    if (push) git(work, 'push', '-q', 'origin', branch)
  }
  const run = (script, ...args) => spawnSync('bash', [join(work, '.claude', 'tools', script), ...args], { cwd: work, env, encoding: 'utf8', timeout: 60000 })
  return { dir, origin, work, env, log, commit, run, logText: () => readFileSync(log, 'utf8'), done: () => rmSync(dir, { recursive: true, force: true }) }
}

const schema = (...lines) => lines.join('\n') + '\n'

test('merge-queue stops before update-branch when develop and the PR both changed the generated files', () => {
  const w = world()
  try {
    w.commit('pr', `${GEN}/schema.json`, schema('a', 'b', 'PR', 'd', 'e'))
    w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'DEV', 'd', 'e'))
    for (const state of ['MERGEABLE BEHIND', 'CONFLICTING DIRTY']) {
      w.env.MQ_STATE = state
      const r = w.run('merge-queue.sh', '7')
      assert.equal(r.status, 1, r.stdout + r.stderr)
      assert.match(r.stdout, /STOP 7: .*both changed web\/src\/lib\/generated/)
      assert.match(r.stdout, /regen-generated\.sh 7/)
    }
    assert.doesNotMatch(w.logText(), /update-branch|pr merge/)
  } finally {
    w.done()
  }
})

test('merge-queue still updates the branch when only develop changed the generated files', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'changed\n')
    w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'DEV', 'd', 'e'))
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(w.logText(), /gh pr update-branch 7/)
    assert.match(r.stdout, /ALL DONE/)
  } finally {
    w.done()
  }
})

test('merge-queue stops on a conflict that touches no generated file, without the regen hint', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'pr\n')
    w.commit('develop', 'web/src/other/x.txt', 'dev\n')
    w.env.MQ_STATE = 'CONFLICTING DIRTY'
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 1)
    assert.match(r.stdout, /STOP 7: CONFLICTING DIRTY/)
    assert.doesNotMatch(r.stdout, /regen-generated/)
  } finally {
    w.done()
  }
})

test('merge-queue leaves a branch that already contains develop alone, whatever the generated files', () => {
  const w = world()
  try {
    w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'DEV', 'd', 'e'))
    git(w.work, 'checkout', '-q', 'pr')
    git(w.work, 'merge', '-q', '--no-edit', 'develop')
    git(w.work, 'push', '-q', 'origin', 'pr')
    w.env.MQ_STATE = 'MERGEABLE CLEAN'
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.doesNotMatch(w.logText(), /update-branch/)
  } finally {
    w.done()
  }
})

test('regen-generated merges develop, regenerates and pushes a merge commit with the regenerated files', () => {
  const w = world()
  try {
    // Adjacent edits: a textual conflict in the generated file, as two PRs adding a command produce.
    w.commit('pr', `${GEN}/schema.json`, schema('a', 'b', 'PR', 'd', 'e'))
    w.commit('pr', 'web/src/other/y.txt', 'pr-only\n')
    w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'DEV', 'd', 'e'))
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    git(w.work, 'fetch', '-q', 'origin')
    const tip = 'origin/pr'
    assert.equal(git(w.work, 'rev-list', '--parents', '-n', '1', tip).split(' ').length, 3, 'a merge commit')
    assert.equal(git(w.work, 'merge-base', '--is-ancestor', 'origin/develop', tip) ?? '', '')
    assert.equal(git(w.work, 'show', `${tip}:${GEN}/schema.json`), 'regenerated')
    assert.equal(git(w.work, 'show', `${tip}:web/src/other/y.txt`), 'pr-only')
    const calls = w.logText().split('\n').filter((l) => l.startsWith('web|'))
    assert.deepEqual(calls, ['web|install --frozen-lockfile', 'web|gen:protocol'])
    assert.equal(git(w.work, 'worktree', 'list').split('\n').length, 1, 'the temporary worktree is gone')
    // The queue now accepts the branch without a build or an update.
    w.env.MQ_STATE = 'MERGEABLE CLEAN'
    const q = w.run('merge-queue.sh', '7')
    assert.equal(q.status, 0, q.stdout + q.stderr)
    assert.doesNotMatch(w.logText(), /update-branch/)
  } finally {
    w.done()
  }
})

test('regen-generated stops on a conflict outside the generated files and pushes nothing', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'pr\n')
    w.commit('develop', 'web/src/other/x.txt', 'dev\n')
    const before = git(w.work, 'rev-parse', 'origin/pr')
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 1)
    assert.match(r.stdout, /conflicts outside web\/src\/lib\/generated/)
    git(w.work, 'fetch', '-q', 'origin')
    assert.equal(git(w.work, 'rev-parse', 'origin/pr'), before)
    assert.doesNotMatch(w.logText(), /gen:protocol/)
    assert.equal(git(w.work, 'worktree', 'list').split('\n').length, 1)
  } finally {
    w.done()
  }
})

test('regen-generated does nothing when the branch already contains develop', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/y.txt', 'pr\n')
    const before = git(w.work, 'rev-parse', 'origin/pr')
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /nothing to do/)
    assert.equal(git(w.work, 'rev-parse', 'origin/pr'), before)
    assert.equal(existsSync(join(w.work, 'web', 'node_modules')), false)
  } finally {
    w.done()
  }
})

test('regen-generated reports a refused push (the branch moved) and leaves no worktree', () => {
  const w = world()
  try {
    w.commit('pr', `${GEN}/schema.json`, schema('a', 'b', 'PR', 'd', 'e'))
    w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'DEV', 'd', 'e'))
    // A hook on origin refuses every push to pr, as a branch that moved in the meantime would.
    const hook = join(w.origin, 'hooks', 'pre-receive')
    writeFileSync(hook, '#!/bin/sh\nexit 1\n')
    chmodSync(hook, 0o755)
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 1)
    assert.match(r.stdout, /push refused/)
    assert.equal(git(w.work, 'worktree', 'list').split('\n').length, 1)
  } finally {
    w.done()
  }
})
