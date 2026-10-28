// Pure logic of scripts/agent/verify.mjs (issue #372): which end-to-end specs a diff needs, how a
// summary reads, how a shared build cache stays bounded. No process is started here.
// The table "file -> specs" is shared with the CI selection (#377): it lives in scripts/ci/e2e-plan.mjs.
import { readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import net from 'node:net'

export { ALL, NONE, RULES, SMOKE, needsWeb, planE2e, selectSpecs, smokeGrep, specFile, specNames } from '../ci/e2e-plan.mjs'

/** `n` distinct free TCP ports on the loopback: the OS hands them out, all held until every one is known. */
export async function freePorts(n, host = '127.0.0.1') {
  const servers = []
  try {
    for (let i = 0; i < n; i++) {
      const server = net.createServer()
      await new Promise((resolve, reject) => {
        server.once('error', reject)
        server.listen(0, host, resolve)
      })
      servers.push(server)
    }
    return servers.map((s) => s.address().port)
  } finally {
    await Promise.all(servers.map((s) => new Promise((r) => s.close(r))))
  }
}

/** True when nothing listens on the port (a connection is refused). */
export function portIsFree(port, host = '127.0.0.1') {
  return new Promise((resolve) => {
    const s = net.createServer()
    s.once('error', () => resolve(false))
    s.listen(port, host, () => s.close(() => resolve(true)))
  })
}

/** Parses `--ports dev,preview,helper`. */
export function parsePorts(text) {
  const ports = String(text).split(',').map((s) => Number(s.trim()))
  if (ports.length !== 3 || ports.some((p) => !Number.isInteger(p) || p < 1 || p > 65535) || new Set(ports).size !== 3) {
    throw new Error(`--ports expects three distinct ports (dev,preview,helper), got "${text}"`)
  }
  return ports
}

/** Size in bytes of a directory tree; stops counting once it exceeds `limit` (a large tree is not walked to the end). */
export function dirSizeUpTo(dir, limit, fs = { readdirSync, statSync }) {
  let total = 0
  const stack = [dir]
  while (stack.length) {
    const d = stack.pop()
    let entries
    try {
      entries = fs.readdirSync(d, { withFileTypes: true })
    } catch {
      continue
    }
    for (const e of entries) {
      const p = join(d, e.name)
      if (e.isDirectory()) stack.push(p)
      else {
        try {
          total += fs.statSync(p).size
        } catch {
          /* vanished while counting */
        }
        if (total > limit) return total
      }
    }
  }
  return total
}

/** Where the build cache shared by every task lives (null when no home is known). Inputs are injected for tests. */
export function sharedTargetDir(env, platform, home = '') {
  const win = platform === 'win32'
  const sep = win ? '\\' : '/'
  const base = win ? env.LOCALAPPDATA || (home && `${home}${sep}AppData${sep}Local`) : env.XDG_CACHE_HOME || (home && `${home}/.cache`)
  return base ? `${base}${sep}faulk-build${sep}target` : null
}

/** "12.3 s" or "2 min 05 s". */
export function duration(ms) {
  const s = ms / 1000
  if (s < 60) return `${s.toFixed(1)} s`
  const min = Math.floor(s / 60)
  const rest = Math.round(s - min * 60)
  return rest === 60 ? `${min + 1} min 00 s` : `${min} min ${String(rest).padStart(2, '0')} s`
}

/**
 * The short summary the script ends with.
 * @param {{name: string, ok: boolean, skipped?: boolean, ms: number, note?: string}[]} steps
 */
export function summary(steps, { base, files, specs, smoke = [], all }) {
  const lines = [`verify against ${base}: ${files.length} changed file(s)`]
  for (const s of steps) {
    const state = s.skipped ? 'skip' : s.ok ? 'ok  ' : 'FAIL'
    lines.push(`  ${state} ${s.name.padEnd(10)} ${s.skipped ? '' : duration(s.ms)}${s.note ? `  ${s.note}` : ''}`.trimEnd())
  }
  lines.push(specs.length ? `  e2e specs${all ? ' (whole suite)' : ''}: ${specs.length > 12 ? `${specs.length} specs` : specs.join(', ')}` : smoke.length ? '  e2e specs: smoke only' : '  e2e specs: none needed')
  if (specs.length && smoke.length) lines.push(`  smoke tests of: ${smoke.join(', ')}`)
  const failed = steps.filter((s) => !s.ok && !s.skipped)
  lines.push(failed.length ? `RESULT: FAIL (${failed.map((s) => s.name).join(', ')})` : `RESULT: OK in ${duration(steps.reduce((t, s) => t + (s.ms || 0), 0))}`)
  return lines.join('\n')
}
