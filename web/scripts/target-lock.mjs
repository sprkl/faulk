// A lock around "cargo build, then copy the artifact". Cargo locks its target directory only while it
// builds; the artifact it leaves (`<target>/<triple>/release/<crate>.wasm`) has the same path for every
// checkout that shares the directory (CARGO_TARGET_DIR, #372), so a second checkout could replace it
// between the first one's build and its copy. Holding this lock for both makes them one step.
//
// The lock is a folder holding the pid of its owner. A waiter takes it over when that process is gone
// (a build killed by force cannot clean up) or when it is older than 15 minutes; the takeover renames
// the folder first, which only one waiter can do.
import { mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const STALE_MS = 15 * 60 * 1000
const WAIT_MS = 30 * 60 * 1000

/** @param {number} ms */
const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms)

/** @param {number} pid */
function alive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (e) {
    return /** @type {NodeJS.ErrnoException} */ (e).code === 'EPERM' // exists, belongs to someone else
  }
}

/**
 * True when the lock folder belongs to nobody any more.
 * @param {string} lock
 */
function abandoned(lock) {
  try {
    if (Date.now() - statSync(lock).mtimeMs > STALE_MS) return true
    const pid = Number(readFileSync(join(lock, 'owner'), 'utf8'))
    return Number.isInteger(pid) && pid > 0 && !alive(pid)
  } catch {
    return false // just released, or its owner has not written its pid yet
  }
}

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
      writeFileSync(join(lock, 'owner'), String(process.pid))
      break
    } catch (e) {
      if (/** @type {NodeJS.ErrnoException} */ (e).code !== 'EEXIST') throw e
      if (abandoned(lock)) {
        try {
          renameSync(lock, `${lock}.dead-${process.pid}`) // one waiter wins this
          rmSync(`${lock}.dead-${process.pid}`, { recursive: true, force: true })
        } catch {
          /* another waiter took it over first */
        }
      } else if (Date.now() - started > WAIT_MS) throw new Error(`build lock held for too long: ${lock}`, { cause: e })
      else sleep(500)
    }
  }
  const release = () => rmSync(lock, { recursive: true, force: true })
  process.once('exit', release) // an interrupted script leaves nothing behind
  try {
    return fn()
  } finally {
    process.removeListener('exit', release)
    release()
  }
}
