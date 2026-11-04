// Which end-to-end specs a change needs: the ONE table "changed file -> specs", shared by the CI
// selection (scripts/ci/select-core.mjs, issue #377) and the local check (scripts/agent/verify.mjs,
// issue #372), which both take the plan from `select`. Pure: no I/O, so the rules are testable.
//
// Three tiers (#533):
//  - area: the specs of the touched areas, whole, plus the core tests of the other specs;
//  - core: the core tests alone (CORE below), for the shared files of the page (stores, app shell,
//    shared i18n, styles…): they replace the whole suite there;
//  - all: the whole suite, for the release pull request, the contracts (protocol, commands, engine
//    sync, generated types), the Playwright and CI configuration, the build inputs, an unknown path,
//    a stale table or an unreadable spec list. A guess never skips a test;
//  - none: nothing a browser test can see.
// A regression outside the area and the core is caught at the latest by the daily release, which runs all.

export const ALL = 'all'
export const NONE = 'none'
export const CORE = 'core'

/** The union of named areas, sorted, without duplicates. */
export const areas = (...names) => {
  for (const n of names) if (!AREA[n]) throw new Error(`no area "${n}"`)
  return [...new Set(names.flatMap((n) => AREA[n]))].sort()
}

// Between the two marker lines below sits the table and nothing else: AREA and RULES, one entry or
// rule per line, comments. A pull request whose only change under scripts/ci/ is an edit of these
// lines runs the core tests and the specs it newly names instead of the whole suite (`tableOnlyChange`);
// any other change of this file, a line outside the table, or a table line of another shape runs everything.
// e2e-table:begin
/** Named areas: the specs that exercise one part of the page. Rules below are unions of them. */
export const AREA = {
  arrangement: ['arrangement', 'arrangement-time', 'arrangement-handles', 'arrangement-view', 'arrangement-draw', 'automation', 'back-to-arrangement', 'clip-names', 'grid-to-arrangement', 'master-pan', 'playhead', 'timegrid', 'ux-quick-wins', 'track-state', 'lane-rows', 'tracks', 'track-order', 'timeline-keyboard', 'discoverability', 'keyboard-create', 'recording', 'piano-roll-follow', 'clip-pool', 'markers', 'context-menu', 'toolbar-overflow', 'device-browser'],
  clips: ['clip-pool'],
  grid: ['clip-names', 'clip-pool', 'grid', 'grid-clips', 'grid-to-arrangement', 'recording'],
  history: ['history', 'history-cost', 'history-names', 'keyboard-create'],
  pianoRoll: ['clip-names', 'piano-roll', 'piano-roll-edit', 'piano-roll-jump', 'playhead', 'typing-mode', 'ux-quick-wins', 'timeline-keyboard', 'keyboard-create', 'recording', 'piano-roll-follow', 'context-menu', 'scale'],
  mixer: ['automation', 'continuous', 'layout-4', 'mixer', 'mixer-sound', 'master-chain', 'master-pan', 'track-state'],
  devices: ['automation', 'continuous', 'device-browser', 'devices', 'presets', 'layout-4', 'master-chain', 'master-pan', 'track-state', 'context-menu'],
  transport: ['all-sound-off', 'transport', 'playhead', 'space-bar', 'track-state', 'discoverability', 'recording', 'stop-to-marker', 'tap-tempo', 'scale'],
  tracks: ['tracks', 'track-order', 'lane-rows', 'discoverability', 'device-browser'],
  deviceBrowser: ['device-browser', 'devices', 'tracks', 'lane-rows', 'discoverability', 'keyboard-all-views'],
  keyboard: ['keyboard', 'keyboard-all-views', 'layout-4', 'live-input', 'typing-mode', 'track-state'],
  export: ['export-audio'],
  storage: ['project-store', 'project-save', 'project-rename', 'project-save-as-name', 'open-project', 'browser-storage', 'storage-commands', 'local-access', 'discoverability', 'open-url', 'audio-import'],
  // Importing audio files (AU-1): the pipeline in the store's Worker, the Clips panel that starts it, the storage it writes; their waveforms (AU-2).
  audio: ['audio-import', 'audio-play', 'clip-pool', 'browser-storage', 'project-store', 'waveform'],
  shortcuts: ['discoverability', 'keyboard-all-views'],
  shortcutsDialog: ['discoverability', 'keyboard-all-views', 'navigation'],
  bottomPanel: ['bottom-panel', 'layout-4', 'ux-quick-wins'],
  topBar: ['helper', 'keyboard-all-views', 'layout-4', 'local-access', 'navigation', 'space-bar', 'track-state', 'discoverability', 'top-bar'],
  midi: ['keyboard', 'live-input', 'recording'],
  keyboardPlayer: ['all-sound-off', 'continuous', 'devices', 'keyboard', 'keyboard-all-views', 'mixer-sound', 'page-dispatcher', 'space-bar', 'tracks', 'typing-mode', 'discoverability'],
  // The landing page's bar and footer are the Journey page's too.
  landing: ['navigation', 'isolation', 'journey', 'landing-links'],
  journey: ['journey'],
  // The shared time grid is drawn by the arrangement and the piano roll: both areas.
  timegrid: ['arrangement', 'arrangement-time', 'arrangement-view', 'automation', 'back-to-arrangement', 'clip-names', 'grid-to-arrangement', 'piano-roll', 'piano-roll-edit', 'piano-roll-jump', 'playhead', 'timegrid', 'lane-rows'],
  // The helper link; the agent scenario (AG-5) drives the real helper's MCP server and the page's control link.
  helper: ['agent-scenario', 'big-project', 'helper', 'local-access'],
  // Typed values in the continuous controls.
  valueEntry: ['value-entry'],
  refusal: ['refused-commands'],
  // What the engine's sound and timing reach: devices, mixer, master, automation, transport, the
  // shipped worklet's cost (engine-mo2), its reports (engine-mo5), the CT-3 prototype that builds
  // crates/engine itself (engine-channel), the export render.
  engine: ['all-sound-off', 'automation', 'big-project', 'continuous', 'devices', 'engine-channel', 'engine-mo2', 'engine-mo5', 'export-audio', 'keyboard', 'master-chain', 'master-pan', 'mixer-sound', 'recording', 'stop-to-marker'],
  // What the project core (project, project-wasm) reaches beyond the core tests: storage, history, refusals.
  project: ['project-store', 'project-save', 'project-rename', 'project-save-as-name', 'open-project', 'browser-storage', 'storage-commands', 'open-url', 'history', 'history-names', 'refused-commands', 'scale'],
}

/**
 * Explicit table "changed file -> end-to-end specs" (spec names without `.spec.ts`). The first rule
 * that matches a file decides: `ALL` (whole suite), `CORE` (shared page file: the core tests only),
 * `NONE` (no browser test sees it), or a list of specs (the area: those specs whole, plus the core
 * tests). A file no rule knows runs the whole suite: an unknown path never skips a test.
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
  [/^web\/(\.prettierrc\.json|\.prettierignore|eslint\.config\.js|\.gitignore|\.vscode\/)/, NONE],
  // The benchmarks have their own Playwright configuration, outside the e2e suite.
  [/^web\/(tests\/bench\/|playwright\.bench\.config\.ts$)/, NONE],
  // The e2e tests themselves: a spec is its own selection, the shared helpers and the configuration run everything.
  [/^web\/tests\/e2e\/support\.ts$/, ALL],
  [/^web\/tests\/e2e\/fixtures\//, ALL],
  [/^web\/tests\/e2e\/([^/]+)\.spec\.ts$/, (m) => [m[1]]],
  [/^web\/(playwright\.config\.ts|package\.json|pnpm-lock\.yaml|pnpm-workspace\.yaml|vite\.config\.ts|svelte\.config\.js|tsconfig\.json|vercel\.json)$/, ALL],
  [/^web\/scripts\//, ALL], // WASM and protocol builds, the production headers
  [/^web\/src\/(app\.html|hooks\.server\.ts)$/, ALL],
  // Contracts: the generated protocol types, the engine sync of the page (worklet, prepare Worker,
  // rings, ABI, reports, the web engine that wires them, and its interface).
  [/^web\/src\/lib\/generated\//, ALL],
  [/^web\/src\/lib\/engine\/(helper-client|helper-copy|native-engine|local-access)\./, AREA.helper],
  // The page side of the agents relay (#97): calls arrive over the helper link and run through the dispatcher.
  [/^web\/src\/lib\/agents\//, [...AREA.helper, ...AREA.history]],
  // The CT-3 prototype (TypeScript and its crate): its own spec builds and runs it.
  [/^web\/src\/lib\/engine\/proto\//, ['engine-channel']],
  [/^web\/src\/lib\/engine\//, ALL],
  // Views.
  [/^web\/src\/views\/arrangement\//, AREA.arrangement],
  [/^web\/src\/views\/clips\/ClipsView\.svelte$/, areas('clips', 'audio')], // it starts the audio import (AU-1)
  [/^web\/src\/views\/clips\//, AREA.clips],
  [/^web\/src\/views\/confirm\//, AREA.helper], // an agent's question before a destructive call (AG-4), asked over the helper link
  [/^web\/src\/views\/grid\//, AREA.grid],
  [/^web\/src\/views\/history\//, AREA.history],
  [/^web\/src\/views\/piano-roll\//, AREA.pianoRoll],
  [/^web\/src\/views\/mixer\//, AREA.mixer],
  [/^web\/src\/views\/devices\//, AREA.devices],
  [/^web\/src\/views\/browser\//, AREA.deviceBrowser],
  [/^web\/src\/views\/transport\//, areas('transport', 'topBar')],
  [/^web\/src\/views\/tracks\//, AREA.tracks],
  [/^web\/src\/views\/keyboard\//, AREA.keyboard],
  [/^web\/src\/views\/shell\/ExportDialog\./, AREA.export],
  [/^web\/src\/views\/shell\/(ProjectMenu|StorageBanners)\./, AREA.storage],
  [/^web\/src\/views\/shell\/ShortcutsDialog\./, AREA.shortcutsDialog],
  [/^web\/src\/views\/shell\/(BottomPanel\.|panel-size\.)/, AREA.bottomPanel],
  [/^web\/src\/views\/shell\/(TopBar|EngineDialog)\./, AREA.topBar],
  // The app shell and the view registry: every view hangs from them.
  [/^web\/src\/views\/(shell\/|registry\.ts$)/, CORE],
  // The shared components: the areas of the files that mount them, transitively (checked against
  // the import graph by e2e-plan.test.mjs).
  [/^web\/src\/lib\/components\/timegrid\//, areas('timegrid', 'arrangement', 'pianoRoll')],
  [/^web\/src\/lib\/components\/ColorPicker\.svelte$/, areas('tracks')],
  [/^web\/src\/lib\/components\/ContextMenu\.svelte$/, areas('mixer', 'tracks', 'devices', 'valueEntry', 'arrangement', 'pianoRoll')],
  [/^web\/src\/lib\/components\/GestureLabel\.svelte$/, areas('arrangement', 'pianoRoll')],
  [/^web\/src\/lib\/components\/GainControl\.svelte$/, areas('mixer', 'tracks')],
  [/^web\/src\/lib\/components\/Knob\.svelte$/, areas('devices', 'valueEntry')],
  [/^web\/src\/lib\/components\/LevelMeter\.svelte$/, areas('arrangement', 'mixer', 'topBar')],
  [/^web\/src\/lib\/components\/LinkMark\.svelte$/, areas('clips', 'grid', 'audio')],
  [/^web\/src\/lib\/components\/Waveform\.svelte$/, areas('clips', 'audio')], // the Clips panel's rows and the /dev/waveform surface (AU-2)
  [/^web\/src\/lib\/components\/Monogram\.svelte$/, areas('landing', 'topBar')],
  [/^web\/src\/lib\/components\/PanControl\.svelte$/, areas('mixer', 'tracks')],
  [/^web\/src\/lib\/components\/ParamControl\.svelte$/, areas('devices', 'valueEntry')],
  [/^web\/src\/lib\/components\/PianoStrip\.svelte$/, areas('keyboard')],
  [/^web\/src\/lib\/components\/ReductionMeter\.svelte$/, areas('devices')],
  [/^web\/src\/lib\/components\/Slider\.svelte$/, areas('mixer', 'tracks', 'devices', 'valueEntry')],
  [/^web\/src\/lib\/components\/ValueField\.svelte$/, areas('mixer', 'tracks', 'devices', 'valueEntry', 'arrangement')],
  [/^web\/src\/lib\/components\/ValueMenu\.svelte$/, areas('mixer', 'tracks', 'devices', 'valueEntry', 'pianoRoll')],
  // A component the table does not name yet (new, or deleted): the core. The pull request that mounts a
  // new one also changes the view that mounts it (its area), and the import-graph test fails until the
  // component gets its own rule.
  [/^web\/src\/lib\/components\//, CORE],
  // The DAW's texts, one file per feature: the areas of the views that show its keys (literal uses
  // checked against the sources by e2e-plan.test.mjs; the device files are read through the device
  // descriptors, by computed keys, so they are placed by hand on the devices area).
  [/^web\/src\/lib\/i18n\/(app|index|lang|feature)\.ts$/, CORE], // the catalogue, the language and its helpers
  [/^web\/src\/lib\/i18n\/(landing)\.ts$/, AREA.landing],
  [/^web\/src\/lib\/i18n\/journey\.ts$/, AREA.landing],
  [/^web\/src\/lib\/i18n\/arrangement\.ts$/, areas('arrangement', 'grid', 'shortcutsDialog')],
  [/^web\/src\/lib\/i18n\/automation\.ts$/, areas('arrangement', 'devices', 'mixer')],
  [/^web\/src\/lib\/i18n\/browser\.ts$/, areas('deviceBrowser', 'tracks')],
  [/^web\/src\/lib\/i18n\/clips\.ts$/, areas('clips', 'audio')],
  [/^web\/src\/lib\/i18n\/confirm\.ts$/, areas('helper', 'history')], // shown by the agents' question (lib/agents/, views/confirm/)
  [/^web\/src\/lib\/i18n\/(devices|presets|synth|drums|echo|reverb|filter|compressor)\.ts$/, areas('devices')],
  [/^web\/src\/lib\/i18n\/controls\.ts$/, areas('mixer', 'tracks', 'devices', 'valueEntry', 'arrangement', 'pianoRoll')],
  [/^web\/src\/lib\/i18n\/engine\.ts$/, areas('arrangement', 'mixer', 'topBar')],
  [/^web\/src\/lib\/i18n\/export\.ts$/, areas('export', 'storage')],
  [/^web\/src\/lib\/i18n\/gesture\.ts$/, areas('arrangement', 'pianoRoll')],
  [/^web\/src\/lib\/i18n\/grid\.ts$/, areas('grid', 'transport', 'topBar')],
  [/^web\/src\/lib\/i18n\/helper\.ts$/, areas('helper', 'topBar')],
  [/^web\/src\/lib\/i18n\/history\.ts$/, areas('history')],
  [/^web\/src\/lib\/i18n\/keys\.ts$/, areas('keyboard', 'keyboardPlayer', 'topBar', 'tracks')],
  [/^web\/src\/lib\/i18n\/layout\.ts$/, areas('bottomPanel', 'topBar')],
  [/^web\/src\/lib\/i18n\/midi\.ts$/, areas('midi', 'topBar')],
  [/^web\/src\/lib\/i18n\/mixer\.ts$/, areas('arrangement', 'devices', 'mixer', 'topBar')],
  [/^web\/src\/lib\/i18n\/piano-roll\.ts$/, areas('arrangement', 'grid', 'pianoRoll')],
  [/^web\/src\/lib\/i18n\/project\.ts$/, areas('storage')],
  [/^web\/src\/lib\/i18n\/refusal\.ts$/, areas('refusal')],
  [/^web\/src\/lib\/i18n\/shell\.ts$/, areas('landing', 'storage', 'topBar')],
  [/^web\/src\/lib\/i18n\/shortcuts\.ts$/, areas('arrangement', 'shortcuts', 'shortcutsDialog', 'topBar')],
  [/^web\/src\/lib\/i18n\/tracks\.ts$/, areas('arrangement', 'grid', 'mixer', 'tracks')],
  [/^web\/src\/lib\/i18n\/transport\.ts$/, areas('transport', 'topBar')],
  [/^web\/src\/lib\/i18n\/scale\.ts$/, areas('transport', 'topBar')], // the key and scale picker of the transport bar (#695)
  // Libraries with one clear audience.
  [/^web\/src\/lib\/shortcuts\.ts$/, AREA.shortcuts],
  [/^web\/src\/lib\/export\//, AREA.export],
  [/^web\/src\/lib\/browser\//, AREA.deviceBrowser],
  [/^web\/src\/lib\/storage\//, AREA.storage],
  [/^web\/src\/lib\/audio\//, AREA.audio],
  [/^web\/src\/lib\/midi\//, AREA.midi],
  [/^web\/src\/lib\/(keyboard\.ts|stores\/keyboard-player)/, AREA.keyboardPlayer],
  [/^web\/src\/lib\/context-menu\.ts$/, ['context-menu']],
  [/^web\/src\/lib\/value-entry\.ts$/, AREA.valueEntry],
  [/^web\/src\/lib\/landing\//, AREA.landing],
  [/^web\/src\/routes\/\[lang=lang\]\/journey\//, AREA.journey],
  [/^web\/src\/routes\/\[lang=lang\]\//, ['navigation', 'isolation']],
  [/^web\/src\/(lib\/journey\/|routes\/journey\/)/, AREA.journey],
  [/^web\/src\/routes\/dev\/waveform\//, ['waveform']],
  [/^web\/src\/routes\/dev\//, ['timegrid']],
  // The landing page at `/`, the layout every page shares, the language parameter of its routes.
  [/^web\/src\/(routes\/\+(layout|page)\.|params\.ts$)/, AREA.landing],
  // Shared files of the page: the core tests (boot, audio, transport, save, export, keyboard, Boost).
  [/^web\/src\/routes\/app\//, CORE],
  [/^web\/src\/lib\/(stores|app|project|meters|styles)\//, CORE],
  [/^web\/src\/lib\/[^/]+\.ts$/, CORE], // the command helpers and small shared modules of lib/
  [/^web\/src\/app\.d\.ts$/, CORE],
  [/^web\/static\//, CORE],
  // The crates the page is built from (select-core.mjs hands over only the files of a crate whose own
  // change reaches engine-wasm, project-wasm, protocol or audio-import-wasm). The contracts: everything.
  [/^crates\/(protocol|commands)\//, ALL],
  [/^crates\/engine\/src\/msg(\.rs$|\/)/, ALL], // the engine sync contract (CT-3)
  [/^crates\/engine-wasm\//, ALL], // the engine's C ABI, the other end of the sync
  [/^crates\/engine\//, AREA.engine],
  [/^crates\/(project|project-wasm)\//, AREA.project],
  // The audio import pipeline (AU-1) and its WebAssembly build, run by the store's Worker.
  [/^crates\/(audio-import|audio-import-wasm)\//, AREA.audio],
  // The helper is no page code, but the browser tests drive the real one (the agent scenario, AG-5) or its stand-in: its area (#668).
  [/^crates\/helper\//, AREA.helper],
  // Crates the page never runs: their tests are the Rust job's.
  [/^crates\/test-support\//, NONE],
  // Anything else under the web app or the crates, and the build inputs: the whole suite.
  [/^(web|crates|modules|\.cargo)\//, ALL],
  [/^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml)$/, ALL],
]
// e2e-table:end

/**
 * The core: the app's backbone, run with every change the page can see (alone for a shared file, with
 * the area's specs otherwise), because a change in one place breaks another through the shared parts
 * the table cannot see (#377: the piano roll's view height once broke the audio export). One entry per
 * whole spec (`{spec}`) or per test (`{spec, title}`: a fragment of the test's title, or of its
 * `describe`, as Playwright prints it; it must appear once in the spec source). About 70 s of tests on
 * one machine (release runs of 2026-11-02); `e2e-plan.test.mjs` keeps every entry honest.
 */
export const CORE_TESTS = [
  // Boot and audio start: cross-origin isolation, shared memory, the AudioWorklet and a Worker, the web engine plays simulated MIDI.
  { spec: 'isolation', title: 'the DAW (/app)' },
  // The engine reports back: meters and play position follow the sound and the transport.
  { spec: 'engine-mo5' },
  // The transport: play, stop, tempo, signature, loop, metronome; a reload during playback.
  { spec: 'transport', title: 'transport, en, 1280 px: play, stop, tempo, signature, loop, metronome' },
  { spec: 'transport', title: 'double clicks and a reload during playback leave the bar consistent' },
  // Save and reopen: into a folder, into the browser (the default, a draft), opening a folder.
  { spec: 'project-store', title: 'edit, save as, edit, reload: the project is found again, and the state tells the truth' },
  { spec: 'browser-storage', title: 'a changed project is kept in the browser, the tab can go, and the draft comes back by itself after a reload' },
  { spec: 'open-project' },
  // The audio export.
  { spec: 'export-audio', title: 'exports the song as a 24-bit WAV file while the song plays' },
  // Keyboard and MIDI dispatch: a key makes sound through the commands, Web MIDI the same way, a refused stream.
  { spec: 'page-dispatcher' },
  { spec: 'keyboard-all-views', title: 'a text field, a keyboard-focused control and a modifier keep their keys' },
  { spec: 'space-bar', title: 'after a mouse click on an arrangement track header, Space plays and pauses' },
  // Every view and panel tab mounts: a shared component or store that breaks one view fails here.
  { spec: 'navigation', title: 'navigation, en, 1280 px: every view, every panel tab, no overflow' },
  // Web <-> Boost: probe, pair and play through the stand-in helper.
  { spec: 'helper' },
]

/** Core entries per spec: `{spec: [title, ...]}`, an empty list meaning the whole spec. */
export const coreBySpec = () => {
  const out = {}
  for (const { spec, title } of CORE_TESTS) {
    out[spec] ??= []
    if (title === undefined) out[spec] = null // whole spec: wins over any title
    else if (out[spec]) out[spec].push(title)
  }
  for (const k of Object.keys(out)) out[k] ??= []
  return out
}

/** Where a file falls: the first rule that matches, or `undefined` (unknown). */
export const ruleFor = (raw) => RULES.find(([re]) => re.test(raw.replaceAll('\\', '/')))

/**
 * @param {string[]} files changed files, repo-relative
 * @param {string[]} existing the spec names present on disk (without `.spec.ts`)
 * @returns {{specs: string[], changed: string[], all: boolean, shared: boolean, reasons: Record<string, string[]>}}
 *   `changed`: the specs among `specs` whose own file changed. `reasons`: per kind (`area`, `shared`,
 *   `all`, `unknown`, `missing`), the files and why.
 */
export function selectSpecs(files, existing) {
  const known = new Set(existing)
  const picked = new Set()
  const changed = new Set()
  const reasons = {}
  let all = false
  let shared = false
  const why = (kind, text) => (reasons[kind] ??= []).push(text)
  for (const raw of files) {
    const f = raw.replaceAll('\\', '/')
    const rule = ruleFor(f)
    // A file no rule knows is a repository file outside the table: run everything.
    if (!rule) {
      all = true
      why('unknown', f)
      continue
    }
    const [re, what] = rule
    if (what === NONE) continue
    if (what === ALL) {
      all = true
      why('all', f)
      continue
    }
    if (what === CORE) {
      shared = true
      why('shared', f)
      continue
    }
    const own = typeof what === 'function' // a spec file: its own selection
    const names = own ? what(re.exec(f)) : what
    let ok = true
    for (const n of names) {
      if (known.has(n)) {
        picked.add(n)
        if (own) changed.add(n)
      }
      else {
        // A stale table, or a deleted spec: never skip a test on a guess.
        all = true
        ok = false
        why('missing', `${f} -> ${n}`)
      }
    }
    if (ok) why('area', f)
  }
  const specs = all ? [...known].sort() : [...picked].sort()
  return { specs, changed: all ? [] : [...changed].sort(), all, shared, reasons }
}

export const specFile = (name) => `tests/e2e/${name}.spec.ts`

/** The spec names of a directory listing (`x.spec.ts` gives `x`). */
export const specNames = (entries) =>
  entries
    .filter((e) => e.endsWith('.spec.ts'))
    .map((e) => e.slice(0, -'.spec.ts'.length))
    .sort()

/** True unless every changed file is one no browser test can see: a file the table does not know counts (fail safe). */
export function needsWeb(files) {
  return files.some((raw) => {
    const rule = ruleFor(raw)
    return !rule || rule[1] !== NONE
  })
}

const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/**
 * The `--grep` that selects exactly the core tests of `specs`: a regular expression over the title
 * Playwright matches (project, file name relative to the test folder, describes and title, joined by
 * spaces). The file name is anchored on a space or a slash, so `arrangement.spec.ts` never matches
 * `grid-to-arrangement.spec.ts`; a whole-spec entry is its file name alone.
 */
export function coreGrep(specs) {
  const by = coreBySpec()
  return specs
    .flatMap((s) => {
      const file = `(^|[ /])${escapeRegExp(`${s}.spec.ts`)}`
      const titles = by[s]
      if (!titles) return []
      return titles.length ? titles.map((t) => `${file} .*${escapeRegExp(t)}`) : [`${file} `]
    })
    .join('|')
}

/** Specs per shard: a machine costs about 2.5 minutes of set-up, a spec about 15 seconds of tests. */
export const SPECS_PER_SHARD = 6
export const MAX_SHARDS = 3

/** How many machines a run of `units` spec files deserves: 1 for a short list, up to 3 for the whole suite. */
export const shardCount = (units) => Math.min(MAX_SHARDS, Math.max(1, Math.ceil(units / SPECS_PER_SHARD)))

/** One line saying why a plan was chosen (for the CI summary and verify.mjs). Lists at most `max` files per kind. */
export function explain(reasons, max = 4) {
  const label = { forced: 'whole suite', unreadable: 'specs unreadable', unknown: 'unknown path', missing: 'stale table', all: 'whole suite for', table: 'table only', shared: 'shared page file', area: 'area of' }
  // What decided the tier comes first: the reasons for the whole suite, then the shared files, then the areas.
  const rank = (k) => (k in label ? Object.keys(label).indexOf(k) : -1)
  return Object.entries(reasons)
    .sort(([a], [b]) => rank(a) - rank(b))
    .map(([kind, list]) => `${label[kind] ?? kind}: ${list.slice(0, max).join(', ')}${list.length > max ? ` (+${list.length - max})` : ''}`)
    .join('; ')
}

/**
 * The tag of the tests that run only with the whole suite (the release pull request, a contract, the
 * configuration): repeated variants (a second language, another width of the same flow) and the
 * performance ceilings, whose strict budgets are checked off CI anyway (#533). A tagged test has
 * `@full` in its title; the area and core runs pass `--grep-invert @full`, the whole suite does not,
 * and a spec whose own file changed runs whole, tags included.
 */
export const FULL_TAG = '@full'

/**
 * What the end-to-end tests have to run for a change.
 * @param {string[]} files changed files, repo-relative (web files, and the crate and build files the caller hands over)
 * @param {string[]} existing the spec names present on disk; empty means unreadable: the whole suite
 * @param {{all?: string, table?: {specs: string[]}}} [options] `all`: the reason the caller forces the
 *   whole suite (release, --full); `table`: the end-to-end table alone changed (`tableOnlyChange`), with
 *   the specs it newly names
 * @returns {{run: boolean, tier: 'none'|'area'|'core'|'all', all: boolean, specs: string[], changed: string[], core: string[], grep: string, shards: number, reasons: Record<string, string[]>, why: string}}
 *   `specs` run whole but for the `@full` tests; `changed` (specs whose own file changed) run whole,
 *   tags included; `core` are other specs of which only the core tests run (`grep`), on the first
 *   machine, unsharded; `run: false`: no browser test.
 */
export function planE2e(files, existing, options = {}) {
  const done = (p) => ({ ...p, why: explain(p.reasons) })
  const whole = (reasons) => done({ run: true, tier: 'all', all: true, specs: existing, changed: [], core: [], grep: '', shards: MAX_SHARDS, reasons })
  if (!existing.length) return done({ run: true, tier: 'all', all: true, specs: [], changed: [], core: [], grep: '', shards: MAX_SHARDS, reasons: { unreadable: ['web/tests/e2e'] } })
  if (options.all) return whole({ forced: [options.all] })
  const sel = selectSpecs(files, existing)
  if (options.table) {
    // The table alone changed: its tests run with the agent scripts; here the core tests and the specs it newly names.
    const missing = options.table.specs.filter((s) => !existing.includes(s))
    if (missing.length) return whole({ ...sel.reasons, missing: missing.map((s) => `table -> ${s}`) })
    sel.specs = [...new Set([...sel.specs, ...options.table.specs])].sort()
    sel.shared = true
    ;(sel.reasons.table ??= []).push(`scripts/ci/e2e-plan.mjs${options.table.specs.length ? ` (newly named: ${options.table.specs.join(', ')})` : ''}`)
  }
  if (sel.all) return whole(sel.reasons)
  if (!sel.specs.length && !sel.shared) return done({ run: false, tier: 'none', all: false, specs: [], changed: [], core: [], grep: '', shards: 0, reasons: sel.reasons })
  // A core entry whose spec is gone is a stale table too: the whole suite, never a silent hole in the core.
  const gone = Object.keys(coreBySpec()).filter((s) => !existing.includes(s))
  if (gone.length) return whole({ ...sel.reasons, missing: gone.map((s) => `CORE_TESTS -> ${s}`) })
  const core = Object.keys(coreBySpec())
    .filter((s) => !sel.specs.includes(s))
    .sort()
  const changed = sel.changed.filter((s) => sel.specs.includes(s))
  const specs = sel.specs.filter((s) => !changed.includes(s))
  // The core run goes on the first machine, whole; the area specs are dealt out between the machines.
  const shards = sel.specs.length ? shardCount(sel.specs.length) : 1
  return done({ run: true, tier: sel.specs.length ? 'area' : 'core', all: false, specs, changed, core, grep: coreGrep(core), shards, reasons: sel.reasons })
}

export const TABLE_FILE = 'scripts/ci/e2e-plan.mjs'
const BEGIN = '// e2e-table:begin'
const END = '// e2e-table:end'

// The shapes a changed line of the table may take: a line comment or a whole one-line block comment,
// an area entry (a list of spec names), or a rule whose verdict is a constant, an area, a union of areas
// or a list of spec names. Anything else (a function, an import, a line that spans several, the middle
// of a block comment, code after a comment opener) is not a table edit.
const TABLE_LINE = [
  /^$/,
  /^\/\/.*$/,
  /^\/\*\*(?:(?!\*\/).)*\*\/$/,
  /^[a-zA-Z]+: \[('[a-z0-9-]+'(, )?)*\],?$/,
  // A regular-expression literal (its body: escapes, character classes, which may hold a slash, and any
  // other character but a slash), then the verdict.
  /^\[\/(?:\\.|\[(?:\\.|[^\]\\])*\]|[^/\\[])+\/[a-z]*, (ALL|NONE|CORE|AREA\.[a-zA-Z]+|areas\(('[a-zA-Z]+'(, )?)+\)|\[('[a-z0-9-]+'(, )?)*\])\],?( \/\/.*)?$/,
]
// A quantified group in a rule's regular expression could backtrack without end on a long path: not a table edit.
const NESTED_QUANTIFIER = /\)[+*{]/

/**
 * The AREA and RULES of a version of this file, from the text between its markers. Only called on a
 * text whose changed lines all have the table's shapes; the rest is the base's own code or code this
 * run already loaded (the head is the running module).
 */
function evalTable(inside) {
  const body = inside.map((l) => l.replace(/^export const /, 'const ')).join('\n')
  const build = new Function(
    'ALL',
    'NONE',
    'CORE',
    // An older table may have no AREA (its lists inline): `typeof` reads an undeclared name safely.
    `const areas = (...names) => { for (const n of names) if (!AREA[n]) throw new Error('no area ' + n); return [...new Set(names.flatMap((n) => AREA[n]))].sort() }\n${body}\nreturn { AREA: typeof AREA === 'undefined' ? {} : AREA, RULES }`,
  )
  return build(ALL, NONE, CORE)
}

/** What a table gives a file: 'all', 'none', 'core' or the list of its specs. */
function verdictOf(rules, file) {
  const rule = rules.find(([re]) => re.test(file))
  if (!rule) return ALL
  const [re, what] = rule
  if (what === ALL || what === NONE || what === CORE) return what
  return typeof what === 'function' ? what(re.exec(file)) : what
}

/** True when `after` selects less for a file than `before` did (the core always rides with a list). */
function weaker(before, after) {
  if (after === ALL || before === NONE) return false
  if (before === ALL) return true
  if (after === NONE) return true
  if (before === CORE) return false // core, or a list (which brings the core along)
  if (after === CORE) return true
  return before.some((s) => !after.includes(s))
}

/**
 * Whether a change of the e2e table file is an edit of the table alone, and which specs it newly names.
 * Strict: the text outside the two marker lines must be identical, each marker must appear once in
 * both versions, every line added or removed between them must have one of the table's shapes, and no
 * file of the repository may get a weaker selection from the new table than from the old one (rules
 * are ordered, the first match wins: a reordered or inserted rule that shadows another is caught here).
 * @param {string} base the file before the change
 * @param {string} head the file after the change
 * @param {string[]} files the repository's files (at least those under web/ and crates/); empty: refused
 * @param {Record<string, string[]>} area the areas of the head (to expand `AREA.x` and `areas('x')`)
 * @returns {{ok: true, specs: string[]} | {ok: false, why: string}} `specs`: the spec names that the
 *   added lines name and the removed lines did not (a superset of the newly named specs)
 */
export function tableOnlyChange(base, head, files, area = AREA) {
  if (!Array.isArray(files) || !files.length) return { ok: false, why: 'no list of repository files to check the table against' }
  const split = (text) => {
    const lines = String(text).replaceAll('\r\n', '\n').split('\n')
    const b = lines.indexOf(BEGIN)
    const e = lines.indexOf(END)
    if (b === -1 || e === -1 || e < b || lines.lastIndexOf(BEGIN) !== b || lines.lastIndexOf(END) !== e) return null
    return { outside: [...lines.slice(0, b), ...lines.slice(e + 1)].join('\n'), inside: lines.slice(b + 1, e) }
  }
  const before = split(base)
  const after = split(head)
  if (!before || !after) return { ok: false, why: 'the table markers are missing or repeated' }
  if (before.outside !== after.outside) return { ok: false, why: 'the file changed outside the table' }
  // Lines added and removed, as multisets (a reordering is no change).
  const count = (lines) => lines.reduce((m, l) => m.set(l, (m.get(l) ?? 0) + 1), new Map())
  const a = count(after.inside)
  const b = count(before.inside)
  const diff = (x, y) => [...x].flatMap(([l, n]) => Array(Math.max(0, n - (y.get(l) ?? 0))).fill(l))
  const added = diff(a, b)
  const removed = diff(b, a)
  for (const line of [...added, ...removed]) {
    const t = line.trim()
    if (!TABLE_LINE.some((re) => re.test(t)) || (t.startsWith('[/') && NESTED_QUANTIFIER.test(t))) return { ok: false, why: `not a table line: ${t.slice(0, 80)}` }
  }
  // The order matters: compare what both tables give every file of the repository.
  let old
  let now
  try {
    old = evalTable(before.inside)
    now = evalTable(after.inside)
  } catch (e) {
    return { ok: false, why: `the table does not evaluate (${e.message})` }
  }
  for (const raw of files) {
    const f = raw.replaceAll('\\', '/')
    if (weaker(verdictOf(old.RULES, f), verdictOf(now.RULES, f))) return { ok: false, why: `the edit narrows the selection of ${f}` }
  }
  // The specs a line names: its quoted names, and the areas it refers to.
  const named = (lines) => {
    const out = new Set()
    for (const raw of lines) {
      const line = raw.trim()
      if (line.startsWith('//') || line.startsWith('*') || line.startsWith('/**')) continue
      for (const m of line.matchAll(/AREA\.([a-zA-Z]+)/g)) for (const s of area[m[1]] ?? []) out.add(s)
      for (const m of line.matchAll(/areas\(([^)]*)\)/g)) for (const n of m[1].matchAll(/'([a-zA-Z]+)'/g)) for (const s of area[n[1]] ?? []) out.add(s)
      const listed = line.replace(/^\[\/(?:\\.|\[(?:\\.|[^\]\\])*\]|[^/\\[])+\/[a-z]*, /, '').replace(/areas\([^)]*\)/g, '')
      for (const m of listed.matchAll(/'([a-z0-9-]+)'/g)) out.add(m[1])
      // An area entry names its own list; its key is no spec.
    }
    return out
  }
  const gone = named(removed)
  return { ok: true, specs: [...named(added)].filter((s) => !gone.has(s)).sort() }
}
