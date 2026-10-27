// Pure logic of scripts/agent/verify.mjs (issue #372): which end-to-end specs a diff needs, how a
// summary reads, how a shared build cache stays bounded. No process is started here.
import { readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import net from 'node:net'

export const ALL = 'all'
export const NONE = 'none'

/**
 * Explicit table "changed file -> end-to-end specs" (spec names without `.spec.ts`). The first rule
 * that matches a file decides. A file no rule knows, or a rule that says `ALL`, runs the whole suite:
 * an unknown path never skips a test. `NONE` is for files no browser test can see.
 * The `dev` project is what the script runs; `preview` (the production build) stays with the release
 * pull request, as in CI (docs/agents/verification.md > CI).
 */
export const RULES = [
  // Nothing a browser sees.
  [/^(docs|\.scratch|\.claude|scripts|\.github)\//, NONE],
  [/^[^/]+\.md$/, NONE],
  [/^web\/(?!tests\/e2e\/).*\.test\.ts$/, NONE],
  [/^web\/.*\.md$/, NONE],
  // The e2e tests themselves: a spec is its own selection, the shared helpers run everything.
  [/^web\/tests\/e2e\/support\.ts$/, ALL],
  [/^web\/tests\/e2e\/fixtures\//, ALL],
  [/^web\/tests\/e2e\/([^/]+)\.spec\.ts$/, (m) => [m[1]]],
  // Views.
  [/^web\/src\/views\/arrangement\//, ['arrangement', 'arrangement-view', 'back-to-arrangement', 'grid-to-arrangement', 'playhead', 'timegrid']],
  [/^web\/src\/views\/grid\//, ['grid', 'grid-clips', 'grid-to-arrangement']],
  [/^web\/src\/views\/piano-roll\//, ['piano-roll', 'piano-roll-edit', 'piano-roll-jump', 'playhead']],
  [/^web\/src\/views\/mixer\//, ['mixer', 'mixer-sound', 'master-chain']],
  [/^web\/src\/views\/devices\//, ['devices', 'master-chain']],
  [/^web\/src\/views\/transport\//, ['transport', 'playhead', 'space-bar']],
  [/^web\/src\/views\/tracks\//, ['tracks', 'track-order']],
  [/^web\/src\/views\/play\//, ['keyboard', 'keyboard-all-views', 'live-input']],
  [/^web\/src\/views\/shell\/ExportDialog\./, ['export-audio']],
  [/^web\/src\/views\/shell\/(ProjectMenu|StorageBanners)\./, ['project-store', 'project-save', 'open-project', 'browser-storage', 'storage-commands', 'local-access']],
  [/^web\/src\/views\/shell\/(BottomPanel\.|panel-size\.)/, ['bottom-panel']],
  // Libraries with one clear audience.
  [/^web\/src\/lib\/export\//, ['export-audio']],
  [/^web\/src\/lib\/storage\//, ['project-store', 'project-save', 'open-project', 'browser-storage', 'storage-commands', 'local-access']],
  [/^web\/src\/lib\/midi\//, ['keyboard', 'live-input']],
  [/^web\/src\/lib\/(keyboard\.ts|stores\/keyboard-player)/, ['keyboard', 'keyboard-all-views', 'space-bar']],
  [/^web\/src\/lib\/(landing\/|i18n\/landing\.ts)/, ['navigation', 'isolation']],
  [/^web\/src\/routes\/\[lang=lang\]\//, ['navigation', 'isolation']],
  [/^web\/src\/routes\/dev\//, ['timegrid']],
  [/^web\/src\/lib\/components\/timegrid\//, ['timegrid', 'arrangement-view']],
  [/^web\/src\/lib\/engine\/(helper-client|helper-copy|native-engine|local-access)\./, ['helper', 'local-access']],
  // Anything else under the web app (stores, engine, i18n, components, styles, generated types,
  // routes, config, lockfile), the Rust crates and the build inputs: the whole suite.
  [/^(web|crates|modules|\.cargo)\//, ALL],
  [/^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml)$/, ALL],
]

/**
 * @param {string[]} files changed files, repo-relative
 * @param {string[]} existing the spec names present on disk (without `.spec.ts`)
 * @returns {{specs: string[], all: boolean, reasons: Record<string, string[]>}}
 */
export function selectSpecs(files, existing) {
  const known = new Set(existing)
  const picked = new Set()
  const reasons = {}
  let all = false
  for (const raw of files) {
    const f = raw.replaceAll('\\', '/')
    const rule = RULES.find(([re]) => re.test(f))
    // A file no rule knows is a repository file outside the table: run everything.
    if (!rule) {
      all = true
      ;(reasons.unknown ??= []).push(f)
      continue
    }
    const [re, what] = rule
    if (what === NONE) continue
    if (what === ALL) {
      all = true
      ;(reasons.shared ??= []).push(f)
      continue
    }
    const names = typeof what === 'function' ? what(re.exec(f)) : what
    for (const n of names) {
      if (known.has(n)) picked.add(n)
      else {
        // A stale table, or a deleted spec: never skip a test on a guess.
        all = true
        ;(reasons.missing ??= []).push(`${f} -> ${n}`)
      }
    }
  }
  const specs = all ? [...known].sort() : [...picked].sort()
  return { specs, all, reasons }
}

export const specFile = (name) => `tests/e2e/${name}.spec.ts`

/** The spec names of a directory listing (`x.spec.ts` gives `x`). */
export const specNames = (entries) => entries.filter((e) => e.endsWith('.spec.ts')).map((e) => e.slice(0, -'.spec.ts'.length)).sort()

/** True when a changed file can change what the web app checks (typecheck, lint, unit tests) see. */
export function needsWeb(files) {
  return files.some((raw) => {
    const f = raw.replaceAll('\\', '/')
    return /^(web|crates|modules|\.cargo)\//.test(f) || /^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml)$/.test(f)
  })
}

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
export function summary(steps, { base, files, specs, all }) {
  const lines = [`verify against ${base}: ${files.length} changed file(s)`]
  for (const s of steps) {
    const state = s.skipped ? 'skip' : s.ok ? 'ok  ' : 'FAIL'
    lines.push(`  ${state} ${s.name.padEnd(10)} ${s.skipped ? '' : duration(s.ms)}${s.note ? `  ${s.note}` : ''}`.trimEnd())
  }
  lines.push(specs.length ? `  e2e specs${all ? ' (whole suite)' : ''}: ${specs.length > 12 ? `${specs.length} specs` : specs.join(', ')}` : '  e2e specs: none needed')
  const failed = steps.filter((s) => !s.ok && !s.skipped)
  lines.push(failed.length ? `RESULT: FAIL (${failed.map((s) => s.name).join(', ')})` : `RESULT: OK in ${duration(steps.reduce((t, s) => t + (s.ms || 0), 0))}`)
  return lines.join('\n')
}
