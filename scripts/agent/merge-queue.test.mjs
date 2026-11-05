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
import { test as nodeTest } from 'node:test'

/**
 * A `bash` that runs scripts. On Windows, `bash` on PATH may be WSL's launcher, which fails without a distribution:
 * probe it, then fall back to Git's own bash (next to `git`). Null when none works.
 */
function findBash() {
  const works = (cmd) => {
    const r = spawnSync(cmd, ['-c', 'echo faulk-ok'], { encoding: 'utf8', timeout: 20000 })
    return r.status === 0 && r.stdout.includes('faulk-ok')
  }
  if (works('bash')) return 'bash'
  if (process.platform === 'win32') {
    const where = spawnSync('where', ['git'], { encoding: 'utf8' })
    for (const line of (where.stdout ?? '').split(/\r?\n/).filter(Boolean)) {
      for (const rel of [['..', 'bin', 'bash.exe'], ['..', '..', 'bin', 'bash.exe']]) {
        const candidate = join(dirname(line), ...rel)
        if (existsSync(candidate) && works(candidate)) return candidate
      }
    }
  }
  return null
}
const BASH = findBash()
const SKIP = BASH ? false : "no usable bash: `bash` is missing or resolves to WSL without a distribution, and Git's bash was not found"
const test = (name, fn) => nodeTest(name, { skip: SKIP }, fn)

const tools = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '.claude', 'tools')
const GEN = 'web/src/lib/generated'

/** A PR body as an agent writes it: French text, then the English commit message section. */
const FENCE = '```'
const SECTION = ['Pourquoi : un test.', '', '## Commit message', '', `${FENCE}text`, 'fix(tools): do the thing', '', 'Explain the change.', FENCE, '', 'Closes #42'].join('\n')

const GH = `#!/usr/bin/env bash
echo "gh $*" >> "$MQ_LOG"
case "$*" in
  *"--json mergeable,mergeStateStatus"*) echo "$MQ_STATE" ;;
  *"--json headRefName"*) echo "$MQ_HEAD" ;;
  *"--json body"*) printf '%s\\n' "$MQ_BODY" ;;
  *"--json closingIssuesReferences"*) printf '%s\\n' "\${MQ_ISSUES-42}" ;;
  *"--json headRefOid"*)
    n=$(cat "$MQ_LOG.n" 2>/dev/null || echo 0); echo $((n + 1)) > "$MQ_LOG.n"
    [ "$n" = 0 ] && echo old || echo new ;;
  "pr update-branch"*) ;;
  "run list"*) echo 1 ;;
  "pr checks"*"--watch"*) ;;
  "pr checks"*"--json name,bucket"*)
    if [ -n "$MQ_CHECKS" ]; then printf '%s\\n' "$MQ_CHECKS"
    else printf 'rust\\t%s\\nweb\\t%s\\n' "\${MQ_BUCKET:-pass}" "\${MQ_BUCKET:-pass}"; fi ;;
  "pr checks"*) ;;
  "pr merge"*)
    for a in "$@"; do echo "arg|$a" >> "$MQ_LOG"; done
    { echo "BODY<<"; cat; echo ">>BODY"; } >> "$MQ_LOG"
    [ -z "$MQ_MERGE_FAIL" ] ;;
  *) exit 99 ;;
esac
`
const PNPM = `#!/usr/bin/env bash
echo "$(basename "$PWD")|$*" >> "$MQ_LOG"
[ "$1" = gen:protocol ] && echo "target|$CARGO_TARGET_DIR" >> "$MQ_LOG"
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
  mkdirSync(join(work, 'scripts', 'agent'), { recursive: true })
  mkdirSync(join(work, 'scripts', 'ci'), { recursive: true })
  for (const f of ['agent/verify-core.mjs', 'ci/e2e-plan.mjs']) copyFileSync(join(tools, '..', '..', 'scripts', f), join(work, 'scripts', f))
  writeFileSync(join(work, GEN, 'schema.json'), ['a', 'b', 'c', 'd', 'e'].join('\n') + '\n')
  writeFileSync(join(work, 'web', 'src', 'other', 'x.txt'), 'x\n')
  git(work, 'add', '-A')
  git(work, 'commit', '-q', '-m', 'base')
  git(work, 'push', '-q', 'origin', 'develop')
  git(work, 'checkout', '-q', '-b', 'pr')
  const log = join(dir, 'log.txt')
  writeFileSync(log, '')
  // The caller's FAULK_* settings (FAULK_SHARED_TARGET=0, ...) must not leak into the scripts under test (#463): tests that need one set it on w.env.
  const inherited = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('FAULK_')))
  const env = {
    ...inherited,
    // Git's bash first, so the stubs' `#!/usr/bin/env bash` does not resolve to WSL either.
    PATH: [...(BASH && BASH !== 'bash' ? [dirname(BASH)] : []), stubs, process.env.PATH].join(delimiter),
    MQ_LOG: log,
    MQ_STATE: 'MERGEABLE BEHIND',
    MQ_HEAD: 'pr',
    MQ_BODY: SECTION,
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
  const run = (script, ...args) => spawnSync(BASH ?? 'bash', [join(work, '.claude', 'tools', script), ...args], { cwd: work, env, encoding: 'utf8', timeout: 60000 })
  return { dir, origin, work, env, log, commit, run, logText: () => readFileSync(log, 'utf8'), done: () => rmSync(dir, { recursive: true, force: true }) }
}

const schema = (...lines) => lines.join('\n') + '\n'

/** The arguments of the stub's `pr merge` call and the body it read on stdin. */
function mergeCall(log) {
  const args = log.split('\n').filter((l) => l.startsWith('arg|')).map((l) => l.slice(4))
  const body = /BODY<<\n([\s\S]*?)>>BODY/.exec(log)?.[1]
  return { args, body, flag: (name) => args[args.indexOf(name) + 1] }
}

test('merge-queue squash-merges with the English subject + (#n), the body, Closes and the branch trailers, pinned to the judged head (#590)', () => {
  const w = world()
  try {
    git(w.work, 'checkout', '-q', 'pr')
    writeFileSync(join(w.work, 'web/src/other/x.txt'), 'changed\n')
    git(w.work, 'add', '-A')
    git(w.work, 'commit', '-q', '-m', 'work\n\nCo-Authored-By: Claude <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_1')
    writeFileSync(join(w.work, 'web/src/other/x.txt'), 'changed again\n')
    git(w.work, 'commit', '-q', '-am', 'more\n\nCo-Authored-By: Claude <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_1')
    git(w.work, 'push', '-q', 'origin', 'pr')
    w.env.MQ_STATE = 'MERGEABLE CLEAN'
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    const m = mergeCall(w.logText())
    assert.ok(m.args.includes('--squash') && !m.args.includes('--merge'), m.args.join(' '))
    assert.equal(m.flag('--subject'), 'fix(tools): do the thing (#7)')
    assert.equal(m.flag('--match-head-commit'), 'old', 'the head whose checks were judged')
    assert.equal(m.flag('--body-file'), '-')
    assert.equal(
      m.body,
      'Explain the change.\n\nCloses #42\n\nCo-Authored-By: Claude <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_1\n',
    )
  } finally {
    w.done()
  }
})

test('merge-queue keeps a subject that already ends with (#n), and adds no Closes or trailer the section already has (#590)', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'changed\n')
    w.env.MQ_STATE = 'MERGEABLE CLEAN'
    w.env.MQ_ISSUES = '42\n43'
    // CRLF line ends, a second section ignored, trailers inside the block moved last.
    w.env.MQ_BODY = ['## Commit message', '', FENCE, 'feat: x (#7)', '', 'Body.', 'Fixes #42', 'Co-Authored-By: A <a@x>', FENCE, '', '## Commit message', FENCE, 'other', FENCE].join('\r\n')
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    const m = mergeCall(w.logText())
    assert.equal(m.flag('--subject'), 'feat: x (#7)')
    assert.equal(m.body, 'Body.\nFixes #42\n\nCloses #43\n\nCo-Authored-By: A <a@x>\n')
  } finally {
    w.done()
  }
})

test('merge-queue stops before updating or merging a PR without an English commit message section (#590)', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'changed\n')
    w.commit('develop', 'web/src/other/y.txt', 'dev\n')
    for (const body of ['Pas de section.\n\nCloses #42', `## Commit message\n\n${FENCE}\n\n${FENCE}\n`, `## Commit message\n\nfix: not in a block\n\n## Next`]) {
      w.env.MQ_BODY = body
      const r = w.run('merge-queue.sh', '7')
      assert.equal(r.status, 1, r.stdout + r.stderr)
      assert.match(r.stdout, /STOP 7: no English commit message/)
    }
    assert.doesNotMatch(w.logText(), /update-branch|pr merge/)
  } finally {
    w.done()
  }
})

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

test('merge-queue waits on a PR blocked by required checks and merges once they pass (#458)', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'changed\n')
    w.env.MQ_STATE = 'MERGEABLE BLOCKED'
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    const log = w.logText()
    const watch = log.indexOf('gh pr checks 7 --watch')
    const read = log.indexOf('gh pr checks 7 --json name,bucket')
    const merge = log.indexOf('gh pr merge 7 --squash')
    assert.ok(watch >= 0 && read > watch && merge > read, `watch, then read, then merge:\n${log}`)
  } finally {
    w.done()
  }
})

test('merge-queue updates a blocked PR that is behind develop before waiting (#458)', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'changed\n')
    w.commit('develop', 'web/src/other/y.txt', 'dev\n')
    w.env.MQ_STATE = 'MERGEABLE BLOCKED'
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    const log = w.logText()
    assert.ok(log.indexOf('gh pr update-branch 7') >= 0 && log.indexOf('gh pr update-branch 7') < log.indexOf('gh pr checks 7 --watch'), log)
    // The pin is the head after the update, the one whose checks were watched (#590).
    assert.equal(mergeCall(log).flag('--match-head-commit'), 'new')
    // The message and the closed issues are read once, before any update or wait (#590).
    assert.ok(log.indexOf('--json body') < log.indexOf('gh pr update-branch 7'), log)
    assert.ok(log.indexOf('--json closingIssuesReferences') < log.indexOf('gh pr update-branch 7'), log)
  } finally {
    w.done()
  }
})

test('merge-queue stops when a required check never reported, even if the others are green (#458)', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'changed\n')
    w.env.MQ_STATE = 'MERGEABLE BLOCKED'
    w.env.MQ_CHECKS = 'web\tpass\nagent-scripts\tpass'
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /STOP 7: required check 'rust' did not report/)
    assert.doesNotMatch(w.logText(), /pr merge/)
  } finally {
    w.done()
  }
})

test('merge-queue stops on a blocked PR whose checks fail, without merging (#458)', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'changed\n')
    w.env.MQ_STATE = 'MERGEABLE BLOCKED'
    w.env.MQ_BUCKET = 'fail'
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /STOP 7: PR checks not green/)
    assert.doesNotMatch(w.logText(), /pr merge/)
  } finally {
    w.done()
  }
})

test('merge-queue stops when the merge itself is refused (protection still blocking) (#458)', () => {
  const w = world()
  try {
    w.commit('pr', 'web/src/other/x.txt', 'changed\n')
    w.env.MQ_STATE = 'MERGEABLE BLOCKED'
    w.env.MQ_MERGE_FAIL = '1'
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /STOP 7: merge failed/)
    assert.doesNotMatch(r.stdout, /ALL DONE/)
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
    assert.match(w.logText(), /gh run list --commit/, 'waits for a run on the current head, as after a regen push')
  } finally {
    w.done()
  }
})

test('regen-generated also regenerates when git merges the generated files cleanly', () => {
  const w = world()
  try {
    w.commit('pr', `${GEN}/schema.json`, schema('PR', 'b', 'c', 'd', 'e'))
    w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'c', 'd', 'DEV'))
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    git(w.work, 'fetch', '-q', 'origin')
    assert.equal(git(w.work, 'show', `origin/pr:${GEN}/schema.json`), 'regenerated')
    assert.equal(git(w.work, 'rev-list', '--parents', '-n', '1', 'origin/pr').split(' ').length, 3)
  } finally {
    w.done()
  }
})

test('regen-generated stops when a conflict mixes generated and other files, pushing nothing', () => {
  const w = world()
  try {
    w.commit('pr', `${GEN}/schema.json`, schema('a', 'b', 'PR', 'd', 'e'))
    w.commit('pr', 'web/src/other/x.txt', 'pr\n')
    w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'DEV', 'd', 'e'))
    w.commit('develop', 'web/src/other/x.txt', 'dev\n')
    const before = git(w.work, 'rev-parse', 'origin/pr')
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 1)
    assert.match(r.stdout, /conflicts outside/)
    git(w.work, 'fetch', '-q', 'origin')
    assert.equal(git(w.work, 'rev-parse', 'origin/pr'), before)
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

/** The two branches edit the generated file; `develop` moves ahead of `pr`. */
function diverged(w) {
  w.commit('pr', `${GEN}/schema.json`, schema('a', 'b', 'PR', 'd', 'e'))
  w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'DEV', 'd', 'e'))
}

test('regen-generated builds into the shared target directory when CARGO_TARGET_DIR is unset', () => {
  const w = world()
  try {
    diverged(w)
    delete w.env.CARGO_TARGET_DIR
    w.env.XDG_CACHE_HOME = join(w.dir, 'cache')
    w.env.LOCALAPPDATA = join(w.dir, 'cache')
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    const line = w.logText().split('\n').find((l) => l.startsWith('target|'))
    assert.match(line, /faulk-build[\\/]target$/)
    assert.ok(line.includes('cache'), line)
  } finally {
    w.done()
  }
})

test('regen-generated leaves cargo to its own target directory when FAULK_SHARED_TARGET=0', () => {
  const w = world()
  try {
    diverged(w)
    delete w.env.CARGO_TARGET_DIR
    w.env.FAULK_SHARED_TARGET = '0'
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.ok(w.logText().split('\n').includes('target|'))
  } finally {
    w.done()
  }
})

test('regen-generated stops with a message when the branch deleted a generated file develop edited', () => {
  const w = world()
  try {
    w.commit('develop', `${GEN}/schema.json`, schema('a', 'b', 'DEV', 'd', 'e'))
    git(w.work, 'checkout', '-q', 'pr')
    git(w.work, 'rm', '-q', `${GEN}/schema.json`)
    git(w.work, 'commit', '-q', '-m', 'pr: delete')
    git(w.work, 'push', '-q', 'origin', 'pr')
    const before = git(w.work, 'rev-parse', 'origin/pr')
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /STOP 7: cannot take the branch's copy/)
    git(w.work, 'fetch', '-q', 'origin')
    assert.equal(git(w.work, 'rev-parse', 'origin/pr'), before)
    assert.equal(git(w.work, 'worktree', 'list').split('\n').length, 1)
  } finally {
    w.done()
  }
})

test('both scripts refuse a head branch that is not on origin (a fork)', () => {
  const w = world()
  try {
    w.env.MQ_HEAD = 'someone:forked'
    const q = w.run('merge-queue.sh', '7')
    assert.equal(q.status, 1)
    assert.match(q.stdout, /origin\/someone:forked not found/)
    const r = w.run('regen-generated.sh', '7')
    assert.equal(r.status, 1)
    assert.match(r.stdout, /origin\/someone:forked not found/)
    assert.doesNotMatch(w.logText(), /update-branch/)
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

/** A PR whose branch is `branch`, pushed with one change, and whose body is `body`. */
function branchWorld(branch, body) {
  const w = world()
  git(w.work, 'checkout', '-q', '-b', branch)
  writeFileSync(join(w.work, 'web/src/other/x.txt'), 'changed\n')
  git(w.work, 'add', '-A')
  git(w.work, 'commit', '-q', '-m', 'work')
  git(w.work, 'push', '-q', 'origin', branch)
  w.env.MQ_HEAD = branch
  w.env.MQ_STATE = 'MERGEABLE CLEAN'
  w.env.MQ_BODY = body
  return w
}
const msgBody = (message, outside = '') => ['Pourquoi.', outside, '', '## Commit message', '', `${FENCE}text`, message, FENCE].join('\n')

test('merge-queue merges a PR whose commit message closes the issue of its branch (#727)', () => {
  for (const [branch, closing] of [['fix/42-thing', 'Closes #42'], ['feat/42-thing', 'Refs #42'], ['spike/42-thing', 'fixes #42']]) {
    const w = branchWorld(branch, msgBody(`fix: x\n\nBody.\n\n${closing}`))
    try {
      const r = w.run('merge-queue.sh', '7')
      assert.equal(r.status, 0, `${branch}: ${r.stdout}${r.stderr}`)
      assert.match(w.logText(), /gh pr merge 7 --squash/)
    } finally {
      w.done()
    }
  }
})

test('merge-queue stops when the commit message does not close the issue of the branch, or names another one (#727)', () => {
  for (const message of ['fix: x\n\nBody without any issue.', 'fix: x\n\nCloses #420', 'fix: x\n\nCloses #682']) {
    // Even a `Closes #42` outside the section does not stand for the message's own.
    const w = branchWorld('fix/42-thing', msgBody(message, 'Closes #42'))
    try {
      const r = w.run('merge-queue.sh', '7')
      assert.equal(r.status, 1, r.stdout + r.stderr)
      assert.match(r.stdout, /STOP 7: branch fix\/42-thing is for issue #42 but its '## Commit message' does not say 'Closes #42'/)
      assert.doesNotMatch(w.logText(), /update-branch|pr merge/)
    } finally {
      w.done()
    }
  }
})

test('merge-queue stops when the commit message also closes another issue, unless the body says so outside the section (#727)', () => {
  const w = branchWorld('fix/560-loop', msgBody('fix: x\n\nCloses #682\nCloses #560', 'Closes #560'))
  try {
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /STOP 7: the '## Commit message' of PR #7 closes #682 but branch fix\/560-loop is for issue #560/)
    assert.doesNotMatch(w.logText(), /pr merge/)
    // A deliberate second issue, closed by a line outside the section, passes.
    w.env.MQ_BODY = msgBody('fix: x\n\nCloses #682\nCloses #560', 'Closes #560\nCloses #682')
    const ok = w.run('merge-queue.sh', '7')
    assert.equal(ok.status, 0, ok.stdout + ok.stderr)
    // A longer number is not the same issue: #6820 outside does not declare #682.
    w.env.MQ_BODY = msgBody('fix: x\n\nCloses #682\nCloses #560', 'Closes #560\nCloses #6820')
    const near = w.run('merge-queue.sh', '7')
    assert.equal(near.status, 1, near.stdout + near.stderr)
  } finally {
    w.done()
  }
})

test('merge-queue does not check a branch outside the convention (#727)', () => {
  for (const branch of ['docs/update-notes', 'release/1.2', 'feat/no-number']) {
    const w = branchWorld(branch, msgBody('docs: x\n\nCloses #999'))
    try {
      const r = w.run('merge-queue.sh', '7')
      assert.equal(r.status, 0, `${branch}: ${r.stdout}${r.stderr}`)
    } finally {
      w.done()
    }
  }
})

test('merge-queue reads several issue numbers on one line, the colon form and CRLF line ends (#727)', () => {
  const w = branchWorld('fix/560-loop', msgBody('fix: x\n\nCloses #560, closes #682', 'Closes #560'))
  try {
    // A second number on the same line is seen, not missed.
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /closes #682 but branch fix\/560-loop/)
    w.env.MQ_BODY = msgBody('fix: x\n\nCloses #12 and #560', 'Closes #560')
    const other = w.run('merge-queue.sh', '7')
    assert.equal(other.status, 1, other.stdout + other.stderr)
    assert.match(other.stdout, /closes #12 but/)
    // Declared outside on one line with the branch's own issue.
    w.env.MQ_BODY = msgBody('fix: x\n\nCloses #560, #682', 'Closes #560 and #682')
    assert.equal(w.run('merge-queue.sh', '7').status, 0)
    // The colon form and CRLF line ends.
    w.env.MQ_BODY = msgBody('fix: x\n\nCloses: #560').replace(/\n/g, '\r\n')
    const crlf = w.run('merge-queue.sh', '7')
    assert.equal(crlf.status, 0, crlf.stdout + crlf.stderr)
    // `Refs` of another issue closes nothing.
    w.env.MQ_BODY = msgBody('fix: x\n\nCloses #560\nRefs #682')
    assert.equal(w.run('merge-queue.sh', '7').status, 0)
  } finally {
    w.done()
  }
})

test('merge-queue sees a closing keyword anywhere in a line, and a branch number with leading zeros (#727)', () => {
  const w = branchWorld('fix/0560-loop', msgBody('fix: x\n\n- Closes #560\n- also fixes #682 and resolves #12', 'Closes #560\n* Closes #682'))
  try {
    // Bulleted and mid-sentence closings: #12 is not declared outside.
    const r = w.run('merge-queue.sh', '7')
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /closes #12 but branch fix\/0560-loop is for issue #560/)
    w.env.MQ_BODY = msgBody('fix: x\n\n- Closes #560\n- also fixes #682 and resolves #12', 'Closes #560\n* Closes #682\nand resolves #12')
    const ok = w.run('merge-queue.sh', '7')
    assert.equal(ok.status, 0, ok.stdout + ok.stderr)
  } finally {
    w.done()
  }
})
