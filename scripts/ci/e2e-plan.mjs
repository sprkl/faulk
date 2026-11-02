// Which end-to-end specs a change needs: the ONE table "changed file -> specs", shared by the CI
// selection (scripts/ci/select-core.mjs, issue #377) and the local check (scripts/agent/verify.mjs,
// issue #372). Pure: no I/O, so the rules are testable. No process is started here.
//
// Policy:
//  - a file the table does not know, a shared file (app shell, stores, engine, protocol, generated
//    types, config), a table that names a spec that no longer exists, or a list of specs that could
//    not be read: the whole suite. A guess never skips a test;
//  - otherwise the specs of the touched areas, plus a small "smoke" set that always runs with them;
//  - nothing a browser test can see: no end-to-end run at all.

export const ALL = 'all'
export const NONE = 'none'

/**
 * Explicit table "changed file -> end-to-end specs" (spec names without `.spec.ts`). The first rule
 * that matches a file decides. A file no rule knows, or a rule that says `ALL`, runs the whole suite:
 * an unknown path never skips a test. `NONE` is for files no browser test can see.
 * The `dev` project is what a pull request against develop and `verify.mjs` run; `preview` (the
 * production build) stays with the release pull request (docs/agents/verification.md > CI).
 */
export const RULES = [
  // The files that decide what runs, and the ones the browser tests are built from: everything.
  [/^\.github\/(workflows\/ci\.yml|actions\/)/, ALL],
  [/^scripts\/ci\//, ALL],
  // Nothing a browser sees.
  [/^(docs|\.scratch|\.claude|scripts|\.github)\//, NONE],
  [/^[^/]+\.md$/, NONE],
  [/^web\/(?!tests\/e2e\/).*\.test\.ts$/, NONE],
  [/^web\/.*\.md$/, NONE],
  // The formatter and lint settings: they change no byte of what the browser runs (the `checks` job runs them).
  [/^web\/(\.prettierrc\.json|\.prettierignore|eslint\.config\.js)$/, NONE],
  // The e2e tests themselves: a spec is its own selection, the shared helpers run everything.
  [/^web\/tests\/e2e\/support\.ts$/, ALL],
  [/^web\/tests\/e2e\/fixtures\//, ALL],
  [/^web\/tests\/e2e\/([^/]+)\.spec\.ts$/, (m) => [m[1]]],
  // Views.
  [/^web\/src\/views\/arrangement\//, ['arrangement', 'arrangement-time', 'arrangement-view', 'automation', 'back-to-arrangement', 'clip-names', 'grid-to-arrangement', 'master-pan', 'playhead', 'timegrid', 'ux-quick-wins', 'track-state', 'lane-rows', 'tracks', 'track-order', 'timeline-keyboard', 'discoverability', 'keyboard-create', 'recording', 'piano-roll-follow']],
  [/^web\/src\/views\/grid\//, ['clip-names', 'grid', 'grid-clips', 'grid-to-arrangement', 'recording']],
  [/^web\/src\/views\/history\//, ['history', 'history-cost', 'keyboard-create']],
  [/^web\/src\/views\/piano-roll\//, ['clip-names', 'piano-roll', 'piano-roll-edit', 'piano-roll-jump', 'playhead', 'typing-mode', 'ux-quick-wins', 'timeline-keyboard', 'keyboard-create', 'recording', 'piano-roll-follow']],
  [/^web\/src\/views\/mixer\//, ['automation', 'continuous', 'layout-4', 'mixer', 'mixer-sound', 'master-chain', 'master-pan', 'track-state']],
  [/^web\/src\/views\/devices\//, ['automation', 'continuous', 'devices', 'layout-4', 'master-chain', 'master-pan', 'track-state']],
  [/^web\/src\/views\/transport\//, ['all-sound-off', 'transport', 'playhead', 'space-bar', 'track-state', 'discoverability', 'recording']],
  [/^web\/src\/views\/tracks\//, ['tracks', 'track-order', 'lane-rows', 'discoverability']],
  [/^web\/src\/views\/keyboard\//, ['keyboard', 'keyboard-all-views', 'layout-4', 'live-input', 'typing-mode', 'track-state']],
  [/^web\/src\/views\/shell\/ExportDialog\./, ['export-audio']],
  [/^web\/src\/views\/shell\/(ProjectMenu|StorageBanners)\./, ['project-store', 'project-save', 'open-project', 'browser-storage', 'storage-commands', 'local-access', 'discoverability', 'open-url']],
  [/^web\/src\/views\/shell\/ShortcutsDialog\./, ['discoverability', 'keyboard-all-views', 'navigation']],
  [/^web\/src\/lib\/(shortcuts\.ts|i18n\/shortcuts\.ts)/, ['discoverability', 'keyboard-all-views']],
  [/^web\/src\/views\/shell\/(BottomPanel\.|panel-size\.)/, ['bottom-panel', 'layout-4', 'ux-quick-wins']],
  [/^web\/src\/views\/shell\/(TopBar|EngineDialog)\./, ['helper', 'keyboard-all-views', 'layout-4', 'local-access', 'navigation', 'space-bar', 'track-state', 'discoverability']],
  // Libraries with one clear audience.
  [/^web\/src\/lib\/export\//, ['export-audio']],
  [/^web\/src\/lib\/storage\//, ['project-store', 'project-save', 'open-project', 'browser-storage', 'storage-commands', 'local-access', 'discoverability', 'open-url']],
  [/^web\/src\/lib\/midi\//, ['keyboard', 'live-input', 'recording']],
  [/^web\/src\/lib\/(keyboard\.ts|stores\/keyboard-player)/, ['all-sound-off', 'continuous', 'devices', 'keyboard', 'keyboard-all-views', 'mixer-sound', 'page-dispatcher', 'space-bar', 'tracks', 'typing-mode', 'discoverability']],
  // The landing page's bar and footer are the Journey page's too.
  [/^web\/src\/lib\/(landing\/|i18n\/landing\.ts)/, ['navigation', 'isolation', 'journey']],
  [/^web\/src\/routes\/\[lang=lang\]\/journey\//, ['journey']],
  [/^web\/src\/routes\/\[lang=lang\]\//, ['navigation', 'isolation']],
  [/^web\/src\/(lib\/journey\/|lib\/i18n\/journey\.ts|routes\/journey\/)/, ['journey']],
  [/^web\/src\/routes\/dev\//, ['timegrid']],
  // The shared time grid is drawn by the arrangement and the piano roll: both areas.
  [/^web\/src\/lib\/components\/timegrid\//, ['arrangement', 'arrangement-time', 'arrangement-view', 'automation', 'back-to-arrangement', 'clip-names', 'grid-to-arrangement', 'piano-roll', 'piano-roll-edit', 'piano-roll-jump', 'playhead', 'timegrid', 'lane-rows']],
  [/^web\/src\/lib\/engine\/(helper-client|helper-copy|native-engine|local-access)\./, ['helper', 'local-access']],
  // Anything else under the web app (stores, engine, i18n, components, styles, generated types,
  // routes, config, lockfile), the Rust crates and the build inputs: the whole suite.
  [/^(web|crates|modules|\.cargo)\//, ALL],
  [/^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml)$/, ALL],
]

/**
 * The smoke set: a few tests of the app's backbone that run with every targeted selection, because
 * a change in one area breaks another through the shared parts the table cannot see (#377: the
 * piano roll's view height once broke the audio export). One entry per test: the spec and a
 * fragment of the test's title (a plain string, matched against the title Playwright prints).
 * Together they cost well under 2 minutes; `e2e-plan.test.mjs` keeps every entry honest (the spec
 * exists, the fragment matches exactly one test of it).
 */
export const SMOKE = [
  // The audio starts: the engine runs in a worklet, under isolation, and plays MIDI.
  { spec: 'isolation', title: 'plays the web engine from simulated Web MIDI under isolation' },
  // The transport plays and stops.
  { spec: 'transport', title: 'transport, en, 1280 px: play, stop, tempo, signature, loop, metronome' },
  // Save, reload, reopen.
  { spec: 'project-store', title: 'edit, save as, edit, reload: the project is found again, and the state tells the truth' },
  { spec: 'open-project', title: 'opening a project folder stops the old project' },
  // The audio export.
  { spec: 'export-audio', title: 'exports the song as a 24-bit WAV file while the song plays' },
]

/** Smoke tests per spec: `{spec: [title, ...]}`. */
export const smokeBySpec = () => {
  const out = {}
  for (const { spec, title } of SMOKE) (out[spec] ??= []).push(title)
  return out
}

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

/** True unless every changed file is one no browser test can see: a file the table does not know counts (fail safe). */
export function needsWeb(files) {
  return files.some((raw) => {
    const rule = RULES.find(([re]) => re.test(raw.replaceAll('\\', '/')))
    return !rule || rule[1] !== NONE
  })
}

const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** The `--grep` that selects exactly the smoke tests of `specs` (a regular expression over the printed title). */
export function smokeGrep(specs) {
  const by = smokeBySpec()
  return specs.flatMap((s) => (by[s] ?? []).map((t) => `${escapeRegExp(`${s}.spec.ts`)}.*${escapeRegExp(t)}`)).join('|')
}

/** Specs per shard: a machine costs about 2.5 minutes of set-up, a spec about 15 seconds of tests. */
export const SPECS_PER_SHARD = 6
export const MAX_SHARDS = 3

/** How many machines a run of `units` spec files deserves: 1 for a short list, up to 3 for the whole suite. */
export const shardCount = (units) => Math.min(MAX_SHARDS, Math.max(1, Math.ceil(units / SPECS_PER_SHARD)))

/**
 * What the end-to-end tests have to run for a change.
 * @param {string[]} files changed files, repo-relative
 * @param {string[]} existing the spec names present on disk; empty means unreadable: the whole suite
 * @param {{all?: boolean}} [options] `all`: something outside the table (CI file, crate, build input) forces the whole suite
 * @returns {{run: boolean, all: boolean, specs: string[], smoke: string[], grep: string, shards: number, reasons: Record<string, string[]>}}
 *   `specs` run whole; `smoke` are other specs of which only the smoke tests run (`grep`); `run: false`: no browser test.
 */
export function planE2e(files, existing, options = {}) {
  if (!existing.length) return { run: true, all: true, specs: [], smoke: [], grep: '', shards: MAX_SHARDS, reasons: { unreadable: ['tests/e2e'] } }
  const sel = options.all ? { all: true, specs: existing, reasons: { forced: ['outside the table'] } } : selectSpecs(files, existing)
  if (sel.all) return { run: true, all: true, specs: sel.specs, smoke: [], grep: '', shards: MAX_SHARDS, reasons: sel.reasons }
  if (!sel.specs.length) return { run: false, all: false, specs: [], smoke: [], grep: '', shards: 0, reasons: sel.reasons }
  const smoke = [...new Set(SMOKE.map((s) => s.spec))].filter((s) => existing.includes(s) && !sel.specs.includes(s)).sort()
  return { run: true, all: false, specs: sel.specs, smoke, grep: smokeGrep(smoke), shards: shardCount(sel.specs.length + (smoke.length ? 1 : 0)), reasons: sel.reasons }
}
