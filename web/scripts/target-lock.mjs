// A lock around "cargo build, then copy the artifact". Cargo locks its target directory only while it
// builds; the artifact it leaves (`<target>/<triple>/release/<crate>.wasm`) has the same path for every
// checkout that shares the directory (CARGO_TARGET_DIR, #372), so a second checkout could replace it
// between the first one's build and its copy. Holding this lock for both makes them one step.
import { mkdirSync, rmSync, statSync } from 'node:fs'
import { join } from 'node:path'

const STALE_MS = 15 * 60 * 1000 // a build that died without releasing
const WAIT_MS = 30 * 60 * 1000

/** @param {number} ms */
const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms)

/**
 * Runs `fn` while holding the lock of the target directory; releases it whatever happens.
 * @template T
 * @param {string} targetDir
 * @param {() => T} fn
 * @returns {T}
 */
export function withTargetLock(targetDir, fn) {
  mkdirSync(targetDir, { recursive: true })
  const lock = join(targetDir, '.faulk-build.lock')
  const started = Date.now()
  for (;;) {
    try {
      mkdirSync(lock) // atomic: fails when it exists
      break
    } catch (e) {
      if (/** @type {NodeJS.ErrnoException} */ (e).code !== 'EEXIST') throw e
      let age
      try {
        age = Date.now() - statSync(lock).mtimeMs
      } catch {
        continue // released while we looked
      }
      if (age > STALE_MS) rmSync(lock, { recursive: true, force: true })
      else if (Date.now() - started > WAIT_MS) throw new Error(`build lock held for too long: ${lock}`, { cause: e })
      else sleep(500)
    }
  }
  try {
    return fn()
  } finally {
    rmSync(lock, { recursive: true, force: true })
  }
}
