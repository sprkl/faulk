import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { join, posix } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { ALL, AREA, CORE, CORE_TESTS, FULL_TAG, FULL_WHEN, MAX_SHARDS, NONE, RULES, SPECS_PER_SHARD, areas, coreBySpec, coreGrep, explain, needsWeb, planE2e, ruleFor, selectSpecs, shardCount, specNames, tableOnlyChange } from './e2e-plan.mjs'

const ROOT = fileURLToPath(new URL('../../', import.meta.url))
const E2E = join(ROOT, 'web/tests/e2e/')
const EXISTING = specNames(readdirSync(E2E))
const plan = (files, options) => planE2e(files, EXISTING, options)
const source = (spec) => readFileSync(`${E2E}${spec}.spec.ts`, 'utf8')
const CORE_SPECS = Object.keys(coreBySpec()).sort()

// The specs that no area rule names: they run when their own file changes, in the core, and on every
// whole-suite run. A new spec must either be named by an area rule or be added here on purpose: this
// list is the enumeration that proves "every spec is placed".
const WHOLE_SUITE_ONLY = []

// ---- the table itself ----

test('every spec on disk is named by an area rule, or is in the core, or is deliberately whole-suite only', () => {
  const named = new Set()
  for (const [, what] of RULES) if (Array.isArray(what)) what.forEach((n) => named.add(n))
  const unplaced = EXISTING.filter((n) => !named.has(n))
  assert.deepEqual(unplaced, [...WHOLE_SUITE_ONLY].sort(), 'a spec is neither in the table nor in WHOLE_SUITE_ONLY')
  for (const n of WHOLE_SUITE_ONLY) assert.ok(CORE_SPECS.includes(n), `${n} is placed by the core only`)
})

test('every spec name the table and the areas list exists on disk', () => {
  for (const [, what] of RULES) if (Array.isArray(what)) for (const n of what) assert.ok(EXISTING.includes(n), `the table names "${n}": no such spec`)
  for (const [area, list] of Object.entries(AREA)) for (const n of list) assert.ok(EXISTING.includes(n), `area ${area} names "${n}": no such spec`)
  assert.throws(() => areas('no-such-area'), /no area/)
})

test('every spec on disk runs whole, its @full tests included, when its own file changes', () => {
  for (const name of EXISTING) {
    const p = plan([`web/tests/e2e/${name}.spec.ts`])
    assert.deepEqual([p.specs, p.changed], [[], [name]], name)
  }
  // Changed and named by an area at once: it runs whole, once.
  const both = plan(['web/tests/e2e/mixer.spec.ts', 'web/src/views/mixer/M.svelte'])
  assert.deepEqual(both.changed, ['mixer'])
  assert.ok(!both.specs.includes('mixer'))
})

test('a file of the store runs the streamed export specs whole, @full included; other files do not (#737)', () => {
  for (const [, names] of FULL_WHEN) for (const n of names) assert.ok(EXISTING.includes(n), `FULL_WHEN names "${n}": no such spec`)
  const store = plan(['web/src/lib/storage/storage.svelte.ts'])
  assert.equal(store.tier, 'area')
  assert.deepEqual(store.changed, ['pack-longtasks', 'pack-stream'])
  // The rest of the area keeps its @full tests to the whole suite.
  assert.ok(store.specs.includes('project-store'))
  assert.ok(!store.specs.includes('pack-stream') && !store.specs.includes('pack-longtasks'))
  assert.match(store.why, /@full specs for: web\/src\/lib\/storage\/storage\.svelte\.ts/)
  // Not for a file outside the store, even one of the same area.
  for (const f of ['web/src/views/shell/StorageBanners.svelte', 'web/src/lib/project/core.ts', 'web/src/views/mixer/M.svelte']) {
    const p = plan([f])
    assert.ok(!p.changed.includes('pack-stream'), f)
  }
  // A name that is no spec is a stale list: the whole suite, never a silent hole.
  const without = EXISTING.filter((n) => n !== 'pack-longtasks')
  const stale = planE2e(['web/src/lib/storage/rpc.ts'], without)
  assert.equal(stale.tier, 'all')
  assert.match(stale.why, /FULL_WHEN web\/src\/lib\/storage\/rpc\.ts -> pack-longtasks/)
})

test('every rule is a verdict or a list of specs', () => {
  for (const [re, what] of RULES) assert.ok(re instanceof RegExp && (what === ALL || what === NONE || what === CORE || Array.isArray(what) || typeof what === 'function'), String(re))
})

test('every file of web/ and crates/ falls on a rule (no web or crate file reaches the unknown fallback by accident)', () => {
  const walk = (d) => readdirSync(d).flatMap((e) => (['node_modules', 'target', '.svelte-kit', 'test-results', 'playwright-report', 'dist', 'build', 'wasm'].includes(e) ? [] : statSync(join(d, e)).isDirectory() ? walk(join(d, e)) : [join(d, e)]))
  const rel = (p) => p.slice(ROOT.length).replaceAll('\\', '/')
  const fallback = RULES.findIndex(([re]) => re.source === '^(web|crates|modules|\\.cargo)\\/')
  assert.ok(fallback > 0)
  const tracked = [...walk(join(ROOT, 'web')), ...walk(join(ROOT, 'crates'))].map(rel)
  const onFallback = tracked.filter((f) => RULES.indexOf(ruleFor(f)) === fallback)
  // What still lands on the fallback: generated or build output a checkout may hold, and nothing else.
  assert.deepEqual(onFallback.filter((f) => !/^web\/src\/lib\/engine\/proto\/|\/(\.svelte-kit|node_modules)\//.test(f) && !f.startsWith('crates/') ), [], 'web files on the whole-suite fallback')
  assert.deepEqual(onFallback.filter((f) => f.startsWith('crates/')).map((f) => f.split('/').slice(0, 2).join('/')).filter((v, i, a) => a.indexOf(v) === i), [], 'a crate with no rule')
})

// ---- the core ----

test('the core: real specs, real tests, each fragment once in its spec, no duplicates', () => {
  const subst = (s) => s.replaceAll('${lang}', 'en').replaceAll('${width}', '1280')
  for (const { spec, title } of CORE_TESTS) {
    assert.ok(EXISTING.includes(spec), `the core names "${spec}": no such spec`)
    if (title !== undefined) assert.equal(subst(source(spec)).split(title).length - 1, 1, `core fragment "${title}" must appear once in ${spec}.spec.ts`)
  }
  assert.equal(new Set(CORE_TESTS.map((s) => `${s.spec}|${s.title}`)).size, CORE_TESTS.length)
})

test('the core covers the backbone the issue names: boot and audio, transport, save and reopen, export, isolation, keyboard dispatch, Web <-> Boost', () => {
  const by = coreBySpec()
  const backbone = {
    'boot, isolation and audio start': ['isolation'],
    transport: ['transport'],
    'save and reopen': ['project-store', 'browser-storage', 'open-project'],
    export: ['export-audio'],
    'keyboard dispatch': ['page-dispatcher', 'keyboard-all-views', 'space-bar'],
    'Web <-> Boost': ['helper'],
    'every view mounts': ['navigation'],
  }
  for (const [what, specs] of Object.entries(backbone)) for (const s of specs) assert.ok(by[s], `${what}: ${s}`)
})

test('core titles stay quotable on a Windows command line (verify.mjs wraps the grep in double quotes)', () => {
  for (const { title } of CORE_TESTS) if (title) assert.ok(!/["%^&<>!]/.test(title), title)
  assert.ok(!/["%&<>!]/.test(coreGrep(CORE_SPECS)), 'the grep holds no character cmd.exe expands inside quotes')
})

test('the core grep matches the titles Playwright matches, and not their neighbours', () => {
  // Playwright matches --grep against "<project> <file relative to testDir> <describes> <title>".
  const re = new RegExp(coreGrep(CORE_SPECS))
  assert.ok(re.test('dev transport.spec.ts transport, en, 1280 px: play, stop, tempo, signature, loop, metronome'))
  assert.ok(!re.test('dev transport.spec.ts transport, fr, 1280 px: play, stop, tempo, signature, loop, metronome'))
  assert.ok(!re.test('dev other.spec.ts transport, en, 1280 px: play, stop, tempo, signature, loop, metronome'))
  // A describe fragment selects its tests, and only its own.
  assert.ok(re.test('dev isolation.spec.ts the DAW (/app) is cross-origin isolated and can share memory'))
  assert.ok(!re.test('dev isolation.spec.ts the landing page /en works isolated: own origin only, fonts, animation'))
  // A whole spec is its file name, anchored: no other file ends with it.
  assert.ok(re.test('dev engine-mo5.spec.ts the playhead advances while the transport plays and stops with it'))
  assert.ok(re.test('dev helper.spec.ts probes, pairs and plays through the helper from the isolated page'))
  assert.ok(!re.test('dev xhelper.spec.ts probes'))
  assert.ok(!re.test('dev not-helper.spec.ts probes'))
  assert.equal(coreGrep([]), '')
  // A title with regular-expression characters stays literal.
  assert.ok(new RegExp(coreGrep(['export-audio'])).test('dev export-audio.spec.ts exports the song as a 24-bit WAV file while the song plays: header'))
  // Only the specs asked for.
  assert.ok(!new RegExp(coreGrep(['transport'])).test('dev helper.spec.ts probes'))
})

// The titles of a spec as Playwright prints them: literal titles as they are, template titles
// rendered with the values the core fragments assume (English, 1280 px); a template that needs any
// other variable cannot be rendered and is left out (a core fragment that only it could match fails).
const VARS = { lang: 'en', width: 1280, locale: 'en-US', size: { width: 1280, height: 800 } }
function renderedTitles(spec) {
  const out = []
  for (const m of source(spec).matchAll(/\btest(?:\.\w+)?\(\s*(`(?:[^`\\]|\\.)*`|'(?:[^'\\]|\\.)*')/g)) {
    const lit = m[1]
    if (lit.startsWith("'")) out.push(lit.slice(1, -1).replaceAll("\\'", "'"))
    else {
      try {
        out.push(new Function(...Object.keys(VARS), `return ${lit}`)(...Object.values(VARS)))
      } catch {
        /* another loop variable */
      }
    }
  }
  return out
}

test('no core test is tagged @full: the core run never loses one to --grep-invert', () => {
  for (const { spec, title } of CORE_TESTS) {
    const titles = renderedTitles(spec)
    if (title === undefined) {
      assert.ok(!source(spec).includes(FULL_TAG), `${spec} is a whole core spec: none of its tests may be ${FULL_TAG}`)
      continue
    }
    const hits = titles.filter((t) => t.includes(title))
    if (!hits.length) {
      // A describe fragment: every test of the spec is a candidate.
      assert.ok(source(spec).includes(`describe('${title}'`), `${spec}: "${title}" matches no rendered test title and no describe`)
      assert.ok(!source(spec).includes(FULL_TAG), `${spec}: the core describe "${title}" sits in a spec with ${FULL_TAG} tests`)
      continue
    }
    for (const t of hits) assert.ok(!t.includes(FULL_TAG), `core test "${t}" is tagged ${FULL_TAG}`)
  }
  // The renderer sees the tags: the transport variants of the core (en, 1280) and the one left to the release (en, 1920).
  assert.ok(renderedTitles('transport').some((t) => t.startsWith('transport, en, 1280 px') && !t.includes(FULL_TAG)))
  const wide = new Function('lang', 'width', `return ${/test\((`transport, [^`]*`)/.exec(source('transport'))[1]}`)('en', 1920)
  assert.ok(wide.endsWith(` ${FULL_TAG}`), wide)
})

test('the @full tests are the ones the lead chose: performance ceilings, second languages, extra widths', () => {
  const tagged = EXISTING.filter((s) => source(s).includes(FULL_TAG)).sort()
  assert.deepEqual(tagged, ['all-sound-off', 'arrangement-time', 'audio-import', 'automation', 'browser-storage', 'grid', 'history-cost', 'keyboard', 'layout-4', 'mixer', 'pack-import', 'pack-longtasks', 'pack-stream', 'project-save', 'project-store', 'refused-commands', 'toolbar-overflow', 'tracks', 'transport'])
  // A spec whose only test is tagged still has a run that passes: the area run passes --pass-with-no-tests.
  assert.ok(!['isolation', 'space-bar'].some((s) => source(s).includes(FULL_TAG)), 'isolation and space-bar stay as they are')
})

// ---- an edit of the table alone (#533) ----

const HEAD = readFileSync(join(ROOT, 'scripts/ci/e2e-plan.mjs'), 'utf8')
const lineOf = (text, start) => text.split('\n').find((l) => l.trimStart().startsWith(start))
// The repository's tracked files: what the CI and verify.mjs hand to the check.
const REPO_FILES = execFileSync('git', ['ls-files'], { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).split('\n').filter(Boolean)
const only = (base, head) => tableOnlyChange(base, head, REPO_FILES)

test('table only: an added spec name in an area is a table edit, and names that spec', () => {
  const before = HEAD.replace("clips: ['clip-pool'],", 'clips: [],')
  assert.deepEqual(only(before, HEAD), { ok: true, specs: ['clip-pool'] })
  // The same file: a table edit with nothing newly named.
  assert.deepEqual(only(HEAD, HEAD), { ok: true, specs: [] })
  // Windows line ends on one side change nothing.
  assert.deepEqual(only(HEAD.replaceAll('\n', '\r\n'), HEAD), { ok: true, specs: [] })
  // A new line comment, a new one-line block comment.
  assert.deepEqual(only(HEAD, HEAD.replace("  clips: ['clip-pool'],", "  // the pool\n  /** the pool */\n  clips: ['clip-pool'],")), { ok: true, specs: [] })
})

test('table only: a new rule names the specs of its areas; a widened rule names what it adds', () => {
  const rule = lineOf(HEAD, '[/^web\\/src\\/lib\\/components\\/PianoStrip')
  const added = HEAD.replace(rule, `${rule}\n  [/^web\\/src\\/lib\\/new-thing\\.ts$/, areas('export', 'midi')],`)
  assert.deepEqual(only(HEAD, added), { ok: true, specs: [...new Set([...AREA.export, ...AREA.midi])].sort() })
  // The removed line named the keyboard area: only "grid" is new.
  const widened = HEAD.replace(rule, `  [/^web\\/src\\/lib\\/components\\/PianoStrip\\.svelte$/, [${[...AREA.keyboard, 'grid'].map((s) => `'${s}'`).join(', ')}]],`)
  assert.deepEqual(only(HEAD, widened), { ok: true, specs: ['grid'] })
})

test('table only, strict: an edit that gives a file a weaker selection is not a table edit (the order counts)', () => {
  const rule = lineOf(HEAD, '[/^web\\/src\\/lib\\/components\\/PianoStrip')
  const shell = lineOf(HEAD, '[/^web\\/src\\/views\\/(shell')
  const cases = {
    // A rule narrowed: its file loses its area.
    narrowed: HEAD.replace(rule, "  [/^web\\/src\\/lib\\/components\\/PianoStrip\\.svelte$/, CORE],"),
    // Two lines swapped: the CORE rule of the shell now shadows the dialogs' areas. Same lines, other order.
    reordered: HEAD.replace(`${lineOf(HEAD, '[/^web\\/src\\/views\\/shell\\/ExportDialog')}\n`, '').replace(shell, `${shell}\n${lineOf(HEAD, '[/^web\\/src\\/views\\/shell\\/ExportDialog')}`),
    // A rule inserted above others that it shadows.
    shadowing: HEAD.replace(lineOf(HEAD, '[/^web\\/src\\/views\\/arrangement\\//'), `  [/^web\\/src\\/views\\//, CORE],\n${lineOf(HEAD, '[/^web\\/src\\/views\\/arrangement\\//')}`),
    // A contract turned into an area.
    contract: HEAD.replace('[/^web\\/src\\/lib\\/generated\\//, ALL],', "[/^web\\/src\\/lib\\/generated\\//, ['helper']],"),
  }
  for (const [what, head] of Object.entries(cases)) {
    assert.notEqual(head, HEAD, `${what}: the case changes the file`)
    const r = only(HEAD, head)
    assert.equal(r.ok, false, what)
    assert.match(r.why, /narrows the selection of /, what)
  }
  // The same order check holds against a multiset trick: the swapped file has the same lines.
  assert.deepEqual([...cases.reordered.split('\n')].sort(), [...HEAD.split('\n')].sort())
})

test('table only, strict: any other change of the file is not a table edit', () => {
  const cases = {
    'outside the table': HEAD.replace('export const SPECS_PER_SHARD = 6', 'export const SPECS_PER_SHARD = 7'),
    'the core': HEAD.replace("{ spec: 'helper' },", "{ spec: 'helper' },\n  { spec: 'grid' },"),
    'a function rule': HEAD.replace("[/^web\\/tests\\/e2e\\/([^/]+)\\.spec\\.ts$/, (m) => [m[1]]],", "[/^web\\/tests\\/e2e\\/([^/]+)\\.spec\\.ts$/, (m) => []],"),
    'code in a rule': HEAD.replace("clips: ['clip-pool'],", "clips: ['clip-pool'].concat(globalThis.x ?? []),"),
    'code hidden next to a regular expression': HEAD.replace('[/^web\\/src\\/views\\/clips\\//, AREA.clips],', '[/^web\\/src\\/views\\/clips\\// ? evil() : /x/, AREA.clips],'),
    'code after a comment opener': HEAD.replace("  clips: ['clip-pool'],", "  /** note */ (RULES.length = 0, 0),\n  clips: ['clip-pool'],"),
    'the middle of a block comment': HEAD.replace("  clips: ['clip-pool'],", "  * 2,\n  clips: ['clip-pool'],"),
    'a nested quantifier': HEAD.replace('[/^web\\/src\\/views\\/clips\\//, AREA.clips],', '[/^web\\/src\\/views\\/(clips\\/)+/, AREA.clips],'),
    'a rule split over lines': HEAD.replace('[/^web\\/src\\/views\\/clips\\//, AREA.clips],', '[/^web\\/src\\/views\\/clips\\//,\n    AREA.clips],'),
    'markers removed': HEAD.replace('// e2e-table:end', ''),
    'markers repeated': HEAD.replace('// e2e-table:end', '// e2e-table:end\n// e2e-table:begin'),
  }
  for (const [what, head] of Object.entries(cases)) {
    assert.notEqual(head, HEAD, `${what}: the case changes the file`)
    const r = only(HEAD, head)
    assert.equal(r.ok, false, what)
    assert.ok(r.why.length > 0, what)
  }
  // A base without the markers (before this change existed): not provable, everything runs.
  assert.equal(only(HEAD.replace('// e2e-table:begin\n', ''), HEAD).ok, false)
  // No list of files to check against: refused.
  for (const files of [undefined, []]) assert.equal(tableOnlyChange(HEAD, HEAD, files).ok, false)
})


test('table only: the plan runs the core tests and the newly named specs, and says why', () => {
  const p = plan([], { table: { specs: ['clip-pool'] } })
  assert.deepEqual([p.tier, p.specs, p.all, p.shards], ['area', ['clip-pool'], false, 1])
  assert.deepEqual(p.core, CORE_SPECS)
  assert.match(p.why, /scripts\/ci\/e2e-plan\.mjs \(newly named: clip-pool\)/)
  const none = plan([], { table: { specs: [] } })
  assert.deepEqual([none.tier, none.specs, none.core], ['core', [], CORE_SPECS])
  // A name that is no spec: stale, the whole suite.
  assert.equal(plan([], { table: { specs: ['no-such-spec'] } }).tier, 'all')
})

test('a whole-spec core entry wins over a title of the same spec', () => {
  for (const [spec, titles] of Object.entries(coreBySpec())) {
    const whole = CORE_TESTS.some((e) => e.spec === spec && e.title === undefined)
    assert.equal(titles.length === 0, whole, spec)
  }
})

// ---- the tiers ----

test('crates/helper gets the helper area, the scenario included, never the whole suite (#668)', () => {
  assert.deepEqual(ruleFor('crates/helper/src/mcp/server.rs')[1], AREA.helper)
  assert.ok(AREA.helper.includes('agent-scenario'))
  const p = plan(['crates/helper/src/tray.rs'])
  assert.deepEqual([p.tier, p.all, p.run], ['area', false, true])
  assert.ok(p.specs.includes('agent-scenario'))
  assert.equal(plan(['crates/test-support/src/lib.rs']).tier, 'none')
})

test('tier none: nothing a browser sees, and a crate the page never runs', () => {
  for (const f of ['docs/a.md', 'scripts/agent/verify.mjs', 'web/src/lib/a.test.ts', 'web/README.md', 'web/.prettierrc.json', 'web/.prettierignore', 'web/eslint.config.js', 'web/tests/bench/engine-swap.bench.ts', 'web/playwright.bench.config.ts', 'CLAUDE.md', '.claude/settings.json', '.github/ISSUE_TEMPLATE/x.yml', 'crates/test-support/src/lib.rs']) {
    const p = plan([f])
    assert.deepEqual([p.tier, p.run, p.specs, p.core, p.shards], ['none', false, [], [], 0], f)
  }
  assert.deepEqual(plan([]).tier, 'none')
})

test('tier area: the specs of the touched area whole, the core tests of the others, on as many machines as the area needs', () => {
  const p = plan(['web/src/views/mixer/MixerFader.svelte'])
  assert.equal(p.tier, 'area')
  assert.deepEqual(p.specs, ['automation', 'continuous', 'layout-4', 'master-chain', 'master-pan', 'mixer', 'mixer-sound', 'track-state'])
  assert.deepEqual(p.core, CORE_SPECS)
  assert.equal(p.all, false)
  assert.equal(p.shards, 2)
  assert.ok(p.grep.length > 0)
  assert.match(p.why, /area of: web\/src\/views\/mixer\/MixerFader\.svelte/)
  // A spec that is in the core and in the area runs whole, not twice.
  const e = plan(['web/src/lib/export/audio-export.svelte.ts'])
  assert.deepEqual(e.specs, ['export-audio'])
  assert.ok(!e.core.includes('export-audio'))
  assert.ok(!new RegExp(e.grep).test('dev export-audio.spec.ts exports the song as a 24-bit WAV file while the song plays'))
  assert.equal(e.shards, 1)
})

test('tier area: an i18n feature file runs the specs of the views that show its keys', () => {
  const p = plan(['web/src/lib/i18n/project.ts'])
  assert.equal(p.tier, 'area')
  assert.deepEqual(p.specs, [...AREA.storage].sort())
  assert.deepEqual(plan(['web/src/lib/i18n/reverb.ts']).specs, [...AREA.devices].sort())
  assert.deepEqual(plan(['web/src/lib/i18n/refusal.ts']).specs, ['refused-commands'])
})

test('tier area: a shared component runs the specs of the views that mount it', () => {
  assert.deepEqual(plan(['web/src/lib/components/PianoStrip.svelte']).specs, [...AREA.keyboard].sort())
  const knob = plan(['web/src/lib/components/Knob.svelte']).specs
  for (const n of ['devices', 'value-entry']) assert.ok(knob.includes(n), n)
  const tg = plan(['web/src/lib/components/timegrid/TimeGrid.svelte']).specs
  for (const n of ['timegrid', 'arrangement-view', 'piano-roll', 'piano-roll-edit', 'piano-roll-jump']) assert.ok(tg.includes(n), n)
})

test('tier area: a crate the page is built from, outside its contracts', () => {
  const engine = plan(['crates/engine/src/devices/reverb.rs'])
  assert.equal(engine.tier, 'area')
  for (const n of ['engine-channel', 'engine-mo2', 'mixer-sound', 'devices']) assert.ok(engine.specs.includes(n), n)
  const project = plan(['crates/project-wasm/src/save.rs'])
  assert.equal(project.tier, 'area')
  for (const n of ['project-store', 'browser-storage', 'history']) assert.ok(project.specs.includes(n), n)
  assert.deepEqual(plan(['web/src/lib/engine/proto/crate/src/lib.rs']).specs, ['engine-channel'])
})

test('tier core: a shared page file runs the core tests alone, on one machine', () => {
  const shared = ['web/src/lib/stores/dispatcher.ts', 'web/src/lib/stores/project.svelte.ts', 'web/src/views/shell/Shell.svelte', 'web/src/views/shell/Placeholder.svelte', 'web/src/views/registry.ts', 'web/src/lib/i18n/app.ts', 'web/src/lib/i18n/lang.ts', 'web/src/lib/app/App.svelte', 'web/src/lib/project/core.ts', 'web/src/lib/styles/tokens.css', 'web/src/lib/track-commands.ts', 'web/src/routes/app/+page.svelte', 'web/static/favicon.svg', 'web/src/app.d.ts']
  for (const f of shared) {
    const p = plan([f])
    assert.deepEqual([p.tier, p.run, p.all, p.specs, p.core, p.shards], ['core', true, false, [], CORE_SPECS, 1], f)
    assert.match(p.why, /shared page file/, f)
  }
  // With an area file: the area's specs, and the core tests of the others; the tier is area.
  const mixed = plan(['web/src/lib/stores/dispatcher.ts', 'web/src/views/tracks/T.svelte'])
  assert.equal(mixed.tier, 'area')
  assert.deepEqual(mixed.specs, [...AREA.tracks].sort())
  assert.match(mixed.why, /shared page file: web\/src\/lib\/stores\/dispatcher\.ts/)
})

test('tier all: contracts, Playwright and CI configuration, build inputs, unknown paths, stale tables, on three machines', () => {
  const whole = ['web/src/lib/generated/protocol.ts', 'web/src/lib/engine/worklet.ts', 'web/src/lib/engine/web-engine.ts', 'web/tests/e2e/support.ts', 'web/tests/e2e/fixtures/measure-rpc.js', 'web/playwright.config.ts', 'web/package.json', 'web/pnpm-lock.yaml', 'web/vite.config.ts', 'web/tsconfig.json', 'web/vercel.json', 'web/scripts/build-wasm.mjs', 'web/src/app.html', '.github/workflows/ci.yml', '.github/actions/web-setup/action.yml', 'scripts/ci/e2e-plan.mjs', 'crates/protocol/src/lib.rs', 'crates/commands/src/track.rs', 'crates/engine/src/msg.rs', 'crates/engine/src/msg/codec.rs', 'crates/engine-wasm/src/lib.rs', 'crates/brand-new/src/lib.rs', 'Cargo.lock', 'modules/x/a.dsp', 'web/some-new-folder/x.ts', 'brand-new-top-folder/x']
  for (const f of whole) {
    const p = plan(['web/src/views/tracks/T.svelte', f])
    assert.equal(p.tier, 'all', f)
    assert.equal(p.all, true, f)
    assert.deepEqual(p.specs, EXISTING, f)
    assert.deepEqual([p.core, p.shards], [[], MAX_SHARDS], f)
    assert.ok(p.why.length > 0, f)
    assert.equal(needsWeb([f]), true, f)
  }
  const stale = planE2e(['web/src/views/mixer/MixerView.svelte'], ['mixer'])
  assert.deepEqual([stale.tier, stale.shards], ['all', MAX_SHARDS])
  assert.match(stale.why, /stale table/)
  // A core entry whose spec is gone: the whole suite, never a silent hole in the core.
  const noCore = planE2e(['web/src/lib/stores/x.ts'], EXISTING.filter((s) => s !== 'helper'))
  assert.equal(noCore.tier, 'all')
  assert.match(noCore.why, /CORE_TESTS -> helper/)
  const unreadable = planE2e(['web/src/views/mixer/MixerView.svelte'], [])
  assert.deepEqual([unreadable.run, unreadable.tier, unreadable.shards], [true, 'all', MAX_SHARDS])
  const forced = plan(['docs/a.md'], { all: 'pull request against main' })
  assert.deepEqual([forced.tier, forced.all], ['all', true])
  assert.match(forced.why, /pull request against main/)
  assert.equal(needsWeb(['.github/workflows/rust.yml']), false)
})

test('the tier of a change is the highest of its files', () => {
  assert.equal(plan(['docs/a.md', 'web/src/lib/stores/x.ts']).tier, 'core')
  assert.equal(plan(['web/src/lib/stores/x.ts', 'web/src/views/grid/G.svelte']).tier, 'area')
  assert.equal(plan(['web/src/lib/stores/x.ts', 'web/src/views/grid/G.svelte', 'web/src/lib/generated/x.ts']).tier, 'all')
})

test('shards: one for a short list, up to three, never zero for a run that happens', () => {
  assert.equal(shardCount(1), 1)
  assert.equal(shardCount(SPECS_PER_SHARD), 1)
  assert.equal(shardCount(SPECS_PER_SHARD + 1), 2)
  assert.equal(shardCount(2 * SPECS_PER_SHARD + 1), 3)
  assert.equal(shardCount(500), MAX_SHARDS)
  for (const f of ['web/src/views/arrangement/A.svelte', 'web/src/views/grid/G.svelte', 'web/src/lib/storage/s.ts', 'web/src/lib/export/e.ts', 'web/src/views/tracks/T.svelte', 'web/src/lib/stores/x.ts']) {
    const p = plan([f])
    assert.ok(p.shards >= 1 && p.shards <= MAX_SHARDS, f)
    assert.equal(p.all, false, f)
  }
})

test('explain says which files chose the tier, briefly', () => {
  assert.equal(explain({ shared: ['a', 'b'] }), 'shared page file: a, b')
  assert.equal(explain({ all: ['1', '2', '3', '4', '5', '6'] }), 'whole suite for: 1, 2, 3, 4 (+2)')
  assert.equal(explain({}), '')
})

test('selectSpecs stays a pure function of the files and the specs on disk', () => {
  assert.deepEqual(selectSpecs(['web/tests/e2e/piano-roll.spec.ts'], EXISTING).specs, ['piano-roll'])
  assert.equal(selectSpecs(['web/src/lib/stores/x.ts'], EXISTING).shared, true)
})

// ---- the finer table, checked against the sources ----

const SRC = join(ROOT, 'web/src')
const walkSrc = (d) => readdirSync(d).flatMap((e) => (statSync(join(d, e)).isDirectory() ? walkSrc(join(d, e)) : [join(d, e)]))
const SOURCES = walkSrc(SRC)
  .filter((f) => /\.(ts|svelte|js)$/.test(f) && !/\.test\.ts$/.test(f))
  .map((f) => ({ path: `web/src/${f.slice(SRC.length + 1).replaceAll('\\', '/')}`, text: readFileSync(f, 'utf8') }))

/** The specs a file's rule gives: a list, [] for NONE or CORE (the core always rides), null for ALL. */
const specsOf = (path) => {
  const rule = ruleFor(path)
  assert.ok(rule, `no rule for ${path}`)
  const [re, what] = rule
  if (what === ALL) return null
  if (what === NONE || what === CORE) return []
  return typeof what === 'function' ? what(re.exec(path)) : what
}

/** Static imports of a source file, resolved to repository paths; an import of web/src it cannot resolve fails the test. */
function importsOf({ path, text }) {
  const out = []
  for (const m of text.matchAll(/(?:import|export)\s[^'"]*?from\s*['"]([^'"]+)['"]|import\(\s*['"]([^'"]+)['"]\s*\)/g)) {
    const spec = m[1] ?? m[2]
    let target
    if (/(^|\/)\$types$/.test(spec)) continue // SvelteKit's generated route types
    if (spec.startsWith('$lib/')) target = `web/src/lib/${spec.slice(5)}`
    else if (spec.startsWith('.')) target = posix.normalize(posix.join(posix.dirname(path), spec))
    else continue // a package
    const candidates = [target, `${target}.ts`, `${target}.js`, `${target}/index.ts`]
    const hit = candidates.find((c) => existsSync(join(ROOT, c)) && statSync(join(ROOT, c)).isFile())
    if (!hit && /^web\/src\/lib\/(generated|wasm)\b|\.wasm|\?/.test(target)) continue // build output, assets
    assert.ok(hit, `${path}: cannot resolve the import "${spec}"`)
    out.push(hit.replaceAll('\\', '/'))
  }
  return out
}

test('every shared component rule covers the specs of every file that mounts it, transitively', () => {
  const importers = new Map()
  for (const s of SOURCES) for (const t of importsOf(s)) (importers.get(t) ?? importers.set(t, []).get(t)).push(s.path)
  const components = SOURCES.map((s) => s.path).filter((p) => p.startsWith('web/src/lib/components/') && p.endsWith('.svelte'))
  assert.ok(components.length >= 10)
  for (const comp of components) {
    const own = specsOf(comp)
    if (own === null) continue
    const seen = new Set()
    const queue = [...(importers.get(comp) ?? [])]
    while (queue.length) {
      const user = queue.pop()
      if (seen.has(user)) continue
      seen.add(user)
      // Through another component (or the time grid's own files), the users of that one count too.
      if (user.startsWith('web/src/lib/components/')) queue.push(...(importers.get(user) ?? []))
      if (user.startsWith('web/src/lib/components/') && ruleFor(user) !== ruleFor(comp)) continue
      const need = specsOf(user)
      if (need === null) continue
      for (const n of need) assert.ok(own.includes(n), `${comp} is mounted by ${user}, whose specs include ${n}`)
    }
  }
})

test('every i18n feature file covers the specs of the files that use its keys (literal keys; device files by hand)', () => {
  const i18n = SOURCES.filter((s) => /^web\/src\/lib\/i18n\/[^/]+\.ts$/.test(s.path))
  for (const file of i18n) {
    const own = specsOf(file.path)
    if (own === null) continue
    const keys = [...new Set([...file.text.matchAll(/^\s+'([a-zA-Z][\w.-]*)':/gm)].map((m) => m[1]))]
    for (const user of SOURCES) {
      if (user.path.startsWith('web/src/lib/i18n/') || user.path.startsWith('web/src/lib/generated/')) continue
      if (!keys.some((k) => user.text.includes(`'${k}'`) || user.text.includes(`"${k}"`) || user.text.includes(`\`${k}`))) continue
      const need = specsOf(user.path)
      if (need === null) continue
      for (const n of need) assert.ok(own.includes(n), `${file.path}: ${user.path} shows its keys, and its specs include ${n}`)
    }
  }
  // Every feature file has its own rule (only the catalogue, the language and the helpers are shared).
  for (const f of i18n) if (specsOf(f.path).length === 0) assert.match(f.path, /\/(app|index|lang|feature)\.ts$/, f.path)
})
