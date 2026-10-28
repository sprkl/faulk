// Claude Code `PostToolUse` hook (Edit, Write, MultiEdit): formats the file the agent just changed, so formatting and
// auto-fixable lint never depend on the agent remembering them (issue #379). Prettier and `eslint --fix` for web/, rustfmt for .rs.
// The rules are in format-core.mjs. Never blocks and never fails: any error, missing tool or timeout is swallowed, the exit code is 0
// and nothing is printed. Works the same on Windows and Linux (no shell: node and rustfmt are started directly).
//
//   node scripts/agent/format-hook.mjs            (the hook payload on stdin)
//   node scripts/agent/format-hook.mjs <file>     (by hand)
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, realpathSync, statSync } from 'node:fs'
import { isAbsolute, join, resolve } from 'node:path'
import { checkoutOf, editionOf, formatSteps } from './format-core.mjs'

// Per step. The hook itself is declared with a longer timeout in .claude/settings.json.
const STEP_TIMEOUT_MS = 20_000

function targetFile(payload, argument) {
  const given = argument ?? payload?.tool_input?.file_path
  if (typeof given !== 'string' || !given) return null
  const base = typeof payload?.cwd === 'string' ? payload.cwd : process.cwd()
  const absolute = isAbsolute(given) ? given : resolve(base, given)
  try {
    if (!statSync(absolute).isFile()) return null
    return realpathSync.native(absolute)
  } catch {
    return null // deleted, or never written
  }
}

function main() {
  const argument = process.argv[2]
  let payload = null
  if (!argument) {
    try {
      payload = JSON.parse(readFileSync(0, 'utf8'))
    } catch {
      return // no payload, or not JSON
    }
  }
  const file = targetFile(payload, argument)
  if (!file) return
  const root = checkoutOf(file, existsSync)
  if (!root) return
  let edition
  try {
    edition = editionOf(readFileSync(join(root, 'Cargo.toml'), 'utf8'))
  } catch {
    /* default edition */
  }
  for (const step of formatSteps({ file, root, exists: existsSync, edition })) {
    // A step that fails (syntax error mid-edit, lint error that cannot be fixed) must not stop the next one, nor the agent.
    spawnSync(step.command, step.args, { cwd: step.cwd, timeout: STEP_TIMEOUT_MS, stdio: 'ignore', windowsHide: true })
  }
}

try {
  main()
} catch {
  /* never fail an edit */
}
process.exitCode = 0
