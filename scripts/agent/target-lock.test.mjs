import assert from 'node:assert/strict'
import { existsSync, mkdtempSync, mkdirSync, rmSync, utimesSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { withTargetLock } from '../../web/scripts/target-lock.mjs'

const temp = () => mkdtempSync(join(tmpdir(), 'faulk-lock-'))

test('the lock is held while the function runs and released after', () => {
  const dir = temp()
  try {
    const lock = join(dir, '.faulk-build.lock')
    const result = withTargetLock(dir, () => {
      assert.equal(existsSync(lock), true)
      return 42
    })
    assert.equal(result, 42)
    assert.equal(existsSync(lock), false)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('the lock is released when the function throws', () => {
  const dir = temp()
  try {
    assert.throws(() => withTargetLock(dir, () => { throw new Error('boom') }), /boom/)
    assert.equal(existsSync(join(dir, '.faulk-build.lock')), false)
    assert.equal(withTargetLock(dir, () => 'again'), 'again')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('a lock left by a dead build is taken over once stale', () => {
  const dir = temp()
  try {
    const lock = join(dir, '.faulk-build.lock')
    mkdirSync(lock)
    const old = new Date(Date.now() - 16 * 60 * 1000)
    utimesSync(lock, old, old)
    assert.equal(withTargetLock(dir, () => 'taken'), 'taken')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
