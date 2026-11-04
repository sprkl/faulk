/**
 * Static guard of the end-to-end specs (#626): the dev server's hooks (`window.__faulkRun`, `__faulkView`,
 * `__faulkHistory`, ...) do not exist in the `preview` project, which serves the production build. A test that
 * calls one must skip itself there, before its first use; the zone level of the CI does not run `preview`, so
 * a miss showed only in the full suite, on another pull request. This reads the specs as text, with no browser.
 *
 * A test counts as guarded when, before its first use of a hook, it holds `skipWithoutDevHooks(page)`
 * (`tests/e2e/support.ts`) or a `test.skip(...)` whose condition tests a hook (`!== 'function'`) or the project's
 * name. A guard outside every test body (a `test.skip` at the top of a file or of a `describe`) covers the whole
 * file. A top-level helper that touches a hook makes the tests that name it users of that hook.
 */

export interface DevHookFinding {
  file: string
  line: number
  test: string
  hook: string
  why: string
}

/** The names the dev server puts on `window`: the keys of the `import.meta.env.DEV` block of the stores. */
export function devHookNames(storesSource: string): string[] {
  const start = storesSource.indexOf('import.meta.env.DEV')
  if (start < 0) return []
  const open = storesSource.indexOf('Object.assign(window, {', start)
  if (open < 0) return []
  let depth = 0
  let end = storesSource.length
  for (let i = open + 'Object.assign'.length; i < storesSource.length; i++) {
    const c = storesSource[i]
    if (c === '{' || c === '(' || c === '[') depth++
    else if ((c === '}' || c === ')' || c === ']') && --depth === 0) {
      end = i
      break
    }
  }
  // A key sits at the first level of the object: its indentation tells (`      __faulkX:`).
  const names = new Set<string>()
  for (const m of storesSource.slice(open, end).matchAll(/^ {6}(__faulk\w+)\s*:/gm)) names.add(m[1])
  return [...names].sort()
}

/** Blanks the comments (line numbers and offsets stay), keeps strings. */
export function stripComments(src: string): string {
  let out = ''
  let i = 0
  while (i < src.length) {
    const c = src[i]
    const n = src[i + 1]
    if (c === '/' && n === '/') {
      while (i < src.length && src[i] !== '\n') {
        out += ' '
        i++
      }
    } else if (c === '/' && n === '*') {
      while (i < src.length && !(src[i] === '*' && src[i + 1] === '/')) {
        out += src[i] === '\n' ? '\n' : ' '
        i++
      }
      out += '  '
      i += 2
    } else if (c === "'" || c === '"' || c === '`') {
      out += c
      i++
      while (i < src.length && src[i] !== c) {
        if (src[i] === '\\') {
          out += src[i]
          i++
        }
        out += src[i]
        i++
      }
      out += c
      i++
    } else {
      out += c
      i++
    }
  }
  return out
}

/** The text of the balanced call that opens at `open` (the index of its `(`), up to and including its `)`. */
function callText(src: string, open: number): string {
  let depth = 0
  let quote = ''
  for (let i = open; i < src.length; i++) {
    const c = src[i]
    if (quote) {
      if (c === '\\') i++
      else if (c === quote) quote = ''
    } else if (c === "'" || c === '"' || c === '`') quote = c
    else if (c === '(') depth++
    else if (c === ')' && --depth === 0) return src.slice(open, i + 1)
  }
  return src.slice(open)
}

const lineOf = (src: string, at: number) => src.slice(0, at).split('\n').length

/** Presence tests (`typeof (window as ...).__faulkRun`) are the guards' own reads, not uses. */
function blankProbes(src: string): string {
  return src.replace(/typeof\s*\(?\s*window\b[^()]*\)?\s*\.__faulk\w+/g, (m) => ' '.repeat(m.length))
}

/** What a test reads to tell the dev server from the production build: `'function'`, the project's name, a presence probe. */
const GUARD_TOKEN = /[!=]==\s*'function'|\bproject\.name\s*[!=]==\s*'(?:dev|preview)'|typeof\s*\(?\s*window\b[^()]*\)?\s*\.__faulk\w+/

interface Span {
  start: number
  end: number
  name: string
  setup: boolean
}

/** A skip that leaves the production build out: the hook is not a function, the project is not `dev` (or is `preview`). */
const SKIP_CONDITION = /!==\s*'function'|\bproject\.name\s*!==\s*'dev'|\bproject\.name\s*===\s*'preview'/

/** The end of the statement that starts at `from`: the first line break outside every bracket. */
function statementEnd(src: string, from: number): number {
  let depth = 0
  for (let i = from; i < src.length; i++) {
    const c = src[i]
    if (c === '(' || c === '[' || c === '{') depth++
    else if (c === ')' || c === ']' || c === '}') depth--
    else if (c === '\n' && depth <= 0) return i
  }
  return src.length
}

/**
 * Where the code tells the two builds apart: `skipWithoutDevHooks(page)`, a `test.skip(...)` whose condition leaves the
 * production build out, or (inside a test body only) an `if` whose condition reads a presence probe or the project's name,
 * or a variable declared from such a read and tested by a later `if`. A bare read of the project's name is not a guard.
 * Outside the test bodies only a `test.skip` written at the left margin counts: a skip inside a `describe` covers that
 * block, not the file.
 */
function guardsIn(src: string, from: number, to: number, inBody: boolean): number[] {
  const at: number[] = []
  const part = src.slice(from, to)
  for (const m of part.matchAll(/\b(skipWithoutDevHooks|test\.skip)\s*(?=\()/g)) {
    const index = from + m.index!
    const call = callText(src, index + m[0].length)
    if (m[1] === 'skipWithoutDevHooks') {
      if (inBody && /^\(\s*page\s*\)$/.test(call)) at.push(index)
    } else if (SKIP_CONDITION.test(call) && (inBody || index === 0 || src[index - 1] === '\n')) at.push(index)
  }
  if (inBody) {
    for (const m of part.matchAll(/\bif\s*(?=\()/g)) {
      if (GUARD_TOKEN.test(callText(src, from + m.index! + m[0].length))) at.push(from + m.index!)
    }
    for (const m of part.matchAll(/\b(?:const|let)\s+(\w+)\s*=/g)) {
      const start = from + m.index!
      if (!GUARD_TOKEN.test(src.slice(start, statementEnd(src, start)))) continue
      const later = part.slice(m.index! + m[0].length)
      if (new RegExp(`\\bif\\s*\\([^)]*\\b${m[1]}\\b`).test(later)) at.push(start)
    }
  }
  return at.sort((x, y) => x - y)
}

export function checkSpec(file: string, source: string, hooks: string[]): DevHookFinding[] {
  const src = stripComments(source)
  const probeless = blankProbes(src)
  const hookRe = new RegExp(`\\b(${hooks.join('|')})\\b`, 'g')

  // Test bodies: from `test(` (or a setup hook) to the line that closes it at the same indentation.
  const spans: Span[] = []
  for (const m of src.matchAll(/^([ \t]*)(test(?:\.(?:only|fixme|fail|slow))?|test\.(?:beforeEach|beforeAll|afterEach|afterAll))\s*\(\s*(['"`])?/gm)) {
    const setup = /before|after/.test(m[2])
    const rest = src.slice(m.index! + m[0].length)
    const title = m[3] ? rest.slice(0, rest.indexOf(m[3])) : m[2]
    const after = src.slice(m.index!)
    const close = new RegExp(`\\n${m[1]}\\}?\\)`).exec(after)
    spans.push({ start: m.index!, end: close ? m.index! + close.index + close[0].length : src.length, name: setup ? m[2] : title, setup })
  }
  if (!spans.length) return []

  // Outside every test body: top-level helpers and describe-level guards.
  const gaps: [number, number][] = []
  let from = 0
  for (const s of spans) {
    if (s.start >= from) gaps.push([from, s.start])
    from = Math.max(from, s.end)
  }
  gaps.push([from, src.length])

  // Top-level helpers that touch a hook, directly or through another helper.
  const decls: { name: string; text: string; guard: number }[] = []
  for (const [a, b] of gaps) {
    for (const p of src.slice(a, b).split(/^(?=(?:export\s+)?(?:async\s+)?(?:function|const|let)\s)/m)) {
      const m = p.match(/^(?:export\s+)?(?:async\s+)?(?:function\s+|const\s+|let\s+)(\w+)/)
      if (m) {
        // A helper that tells the two builds apart (a skip, a `project.name` branch) guards itself, and its callers.
        const text = p.slice(m[0].length)
        const guard = guardsIn(text, 0, text.length, true)[0] ?? -1
        decls.push({ name: m[1], text: blankProbes(text), guard })
      }
    }
  }
  const users = new Set<string>()
  for (let changed = true; changed;) {
    changed = false
    for (const d of decls) {
      if (users.has(d.name)) continue
      const first = Math.min(...[hookRe, ...[...users].map((u) => new RegExp(`\\b${u}\\b`))].map((re) => d.text.search(re)).map((i) => (i < 0 ? Infinity : i)))
      if (first !== Infinity && (d.guard < 0 || d.guard > first)) {
        users.add(d.name)
        changed = true
      }
    }
  }

  // A guard outside every test body covers the file.
  const fileGuard = gaps.some(([a, b]) => guardsIn(src, a, b, false).length > 0)

  const findings: DevHookFinding[] = []
  for (const s of spans) {
    if (s.setup && /after/.test(s.name)) continue
    const body = probeless.slice(s.start, s.end)
    // A helper is named in code, not in the test's title or a message.
    const code = body.replace(/(['"`])(?:\\.|(?!\1)[^\\])*\1/g, (m) => m[0] + ' '.repeat(m.length - 2) + m[0])
    const uses: { at: number; hook: string }[] = []
    for (const m of body.matchAll(hookRe)) uses.push({ at: s.start + m.index!, hook: m[1] })
    for (const u of users)
      for (const m of code.matchAll(new RegExp(`\\b${u}\\b`, 'g')))
        uses.push({ at: s.start + m.index!, hook: `${u}() (a helper of the file that calls a dev hook)` })
    if (!uses.length || fileGuard) continue
    uses.sort((a, b) => a.at - b.at)
    const first = uses[0]
    const guards = guardsIn(src, s.start, s.end, true)
    // A call to a helper that skips for the preview project is a guard, where the call stands.
    for (const d of decls) {
      if (d.guard < 0) continue
      const call = new RegExp(`\\b${d.name}\\s*\\(`).exec(code)
      if (call) guards.push(s.start + call.index)
    }
    guards.sort((x, y) => x - y)
    if (!guards.length) {
      findings.push({ file, line: lineOf(src, first.at), test: s.name, hook: first.hook, why: 'no skip for the preview project' })
    } else if (guards[0] > first.at) {
      findings.push({ file, line: lineOf(src, first.at), test: s.name, hook: first.hook, why: `used before the skip at line ${lineOf(src, guards[0])}` })
    }
  }
  return findings
}

export const describeFinding = (f: DevHookFinding) => `${f.file}:${f.line} "${f.test}" uses ${f.hook}: ${f.why}`
