// What formats a file, decided without touching the disk beyond `exists` (injected), so the rules are testable.
// Used by the PostToolUse hook (format-hook.mjs) and by `verify.mjs --fix`.
//
//   web/**       eslint --fix for the files ESLint reads, then Prettier (the config of web/): Prettier always has the last word on layout,
//                so a file is never left in a state `prettier --check` (the CI) rejects;
//   **/*.rs      rustfmt.
//
// Everything else (docs, YAML, TOML, generated files, dependencies) is left alone.
import { dirname, join, relative, resolve, sep } from 'node:path'

const PRETTIER_EXTENSIONS = new Set(['.ts', '.js', '.mjs', '.cjs', '.svelte', '.css', '.json', '.html'])
const ESLINT_EXTENSIONS = new Set(['.ts', '.js', '.mjs', '.svelte'])
// Never touched, whatever their extension: dependencies, build output, generated code, and the test fixtures (raw source text that the
// tests wrap and run in a page: a formatter's leading semicolon breaks them; web/.prettierignore says the same for `pnpm format`).
const SKIPPED_SEGMENTS = new Set(['node_modules', 'target', 'dist', '.svelte-kit', 'wasm', 'generated', 'test-results', 'playwright-report', 'fixtures'])

/** Last extension of a path, lowercased ('' when none). */
export function extensionOf(file) {
  const name = file.slice(file.lastIndexOf(sep) + 1)
  const dot = name.lastIndexOf('.')
  return dot <= 0 ? '' : name.slice(dot).toLowerCase()
}

/**
 * The checkout a file belongs to: the closest directory above it that holds `.git` (a directory, or a file in a worktree).
 * @param {string} file absolute path
 * @param {(path: string) => boolean} exists
 * @returns {string | null}
 */
export function checkoutOf(file, exists) {
  let dir = dirname(file)
  for (;;) {
    if (exists(join(dir, '.git'))) return dir
    const parent = dirname(dir)
    if (parent === dir) return null
    dir = parent
  }
}

/**
 * The commands that format one file. Empty when the file is none of the tool's business, or when the tools are not installed
 * (`web/node_modules` absent): the hook then does nothing instead of failing.
 * @param {object} input
 * @param {string} input.file absolute path of the file
 * @param {string} input.root absolute path of the checkout
 * @param {(path: string) => boolean} input.exists
 * @param {string} [input.node] the node executable
 * @param {string} [input.edition] Rust edition for rustfmt
 * @returns {{ name: string, command: string, args: string[], cwd: string }[]}
 */
export function formatSteps({ file, root, exists, node = process.execPath, edition = '2024' }) {
  const rel = relative(root, resolve(file))
  if (!rel || rel.startsWith('..') || resolve(rel) === rel) return [] // outside the checkout
  const segments = rel.split(sep)
  if (segments.some((s) => SKIPPED_SEGMENTS.has(s))) return []
  const ext = extensionOf(file)
  const steps = []

  if (segments[0] === 'web') {
    const web = join(root, 'web')
    const prettier = join(web, 'node_modules', 'prettier', 'bin', 'prettier.cjs')
    const eslint = join(web, 'node_modules', 'eslint', 'bin', 'eslint.js')
    if (ESLINT_EXTENSIONS.has(ext) && exists(eslint)) {
      steps.push({ name: 'eslint', command: node, args: [eslint, '--fix', '--no-warn-ignored', '--quiet', file], cwd: web })
    }
    if (PRETTIER_EXTENSIONS.has(ext) && exists(prettier)) {
      steps.push({ name: 'prettier', command: node, args: [prettier, '--write', '--ignore-unknown', '--log-level', 'silent', file], cwd: web })
    }
  } else if (ext === '.rs') {
    steps.push({ name: 'rustfmt', command: 'rustfmt', args: ['--edition', edition, file], cwd: root })
  }
  return steps
}

/** The Rust edition of the workspace, from the text of the root Cargo.toml. */
export function editionOf(cargoToml) {
  return /^edition\s*=\s*"(\d{4})"/m.exec(cargoToml)?.[1] ?? '2024'
}
