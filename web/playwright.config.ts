import { defineConfig } from '@playwright/test'
import { e2eHelperPort } from './scripts/e2e-helper-port'

/**
 * End-to-end tests in a real browser, against the dev server (`pnpm dev`) and the built site under
 * the static preview of a fresh build (`pnpm build && pnpm preview`), on their own ports (5180, 4180) so
 * that a server left running on 5173 or 4173 does not get in the way. The preview serves
 * the same headers as production (`vercel.json`, mirrored by `scripts/vercel-headers.ts`). The dev
 * server skips the wasm build of `pnpm dev`: `pnpm build` makes it, and two concurrent copies of
 * the same file could race.
 *
 * The browser is Playwright's Chromium; `FAULK_E2E_CHANNEL=chrome` or `msedge` runs the installed
 * Chrome or Edge instead. One worker: the helper test listens on the checkout's own helper port (#505). The
 * servers are always started here, never reused: a server already on those ports may come from
 * another checkout, and the run then fails on the busy port instead of testing other code.
 */
const channel = process.env.FAULK_E2E_CHANNEL || undefined
// The page and the stand-in helper of the helper test agree on this port (`tests/e2e/support.ts`): `FAULK_E2E_HELPER_PORT`, else one
// of this checkout's own (derived from its path, #505), so two checkouts running the suite never share it and the installed helper's
// 47800 is never the stand-in's.
process.env.VITE_FAULK_HELPER_PORT = String(e2eHelperPort())

// `FAULK_E2E_DEV_PORT` and `FAULK_E2E_PREVIEW_PORT` move the two servers when another checkout holds 5180 or 4180.
const devPort = Number(process.env.FAULK_E2E_DEV_PORT) || 5180
const previewPort = Number(process.env.FAULK_E2E_PREVIEW_PORT) || 4180

// `FAULK_E2E_PROJECTS=dev` (or `preview`) runs one project and starts only its server: CI runs `dev` on
// a task pull request and both on the pull request against main. Unset: both.
const wanted = (process.env.FAULK_E2E_PROJECTS || 'dev,preview').split(',').map((s) => s.trim())
for (const name of wanted) if (name !== 'dev' && name !== 'preview') throw new Error(`FAULK_E2E_PROJECTS: unknown project "${name}"`)

/**
 * The app opens on the arrangement (#475), or on the view used last. Most specs start on the launch grid, as a visitor
 * who used it last would (the Instrument view they once started on is gone, #472): it draws no time grid and holds no
 * mixer strip, so the testids of the roll and of the devices are unique there. `ux-quick-wins.spec.ts` starts fresh.
 */
const onGrid = (origin: string) => ({ cookies: [], origins: [{ origin, localStorage: [{ name: 'faulk.view', value: 'grid' }] }] })

/**
 * The specs the `preview` project runs (founder's decision of 2026-11-12, #848): the ones that see what the production
 * build changes (the minified bundle, the WASM modules with their names stripped, the headers of `vercel.json`): the
 * engine and its worklet, the Workers, cross-origin isolation, storage, the exports, the landing and Journey pages, and
 * the other specs of the e2e audit of that day. The interface specs (tracks, grid, transport, mixer, history, piano roll
 * editing...) run on `dev` only; a new spec runs on `dev` only unless it is added here. Every test of these specs runs,
 * `@full` ones included, plus one test of `mixer.spec.ts`, the render cost of 32 strips (`PREVIEW_MIXER_TEST`). This
 * list is the only place the subset is written: CI deals out what `playwright test --list` gives (scripts/ci/e2e-shard.mjs).
 */
export const PREVIEW_SPECS = [
  'arrangement',
  'audio-clips',
  'browser-storage',
  'devices',
  'engine-mo2',
  'engine-mo5',
  'export-audio',
  'helper',
  'isolation',
  'journey',
  'landing-links',
  'master-chain',
  'navigation',
  'open-url',
  'output-stall',
  'pack-import',
  'piano-roll',
  'project-store',
  'storage-commands',
]
/** The one test of `mixer.spec.ts` the `preview` project runs: a fragment of its title. */
export const PREVIEW_MIXER_TEST = '32 strips with every meter moving: render cost'

export default defineConfig({
  testDir: 'tests/e2e',
  // Above the boot ceiling (`BOOT` in tests/e2e/support.ts, 30 s): a default test of 30 s would cut it short (#539).
  // A test that waits for the engine twice or does more sets its own.
  timeout: 90_000,
  workers: 1,
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  // `FAULK_E2E_JSON`: also a JSON report at that path (CI, one per command): the measures `scripts/ci/e2e-durations.mjs`
  // rebuilds the machines' plan from (#848).
  reporter: [
    ['list'],
    ...(process.env.CI ? [['github'] as const] : []),
    ...(process.env.FAULK_E2E_JSON ? [['json', { outputFile: process.env.FAULK_E2E_JSON }] as const] : []),
  ],
  use: { channel, locale: 'en-US', trace: 'retain-on-failure' },
  projects: [
    { name: 'dev', use: { baseURL: `http://localhost:${devPort}`, storageState: onGrid(`http://localhost:${devPort}`) } },
    {
      name: 'preview',
      testMatch: [...PREVIEW_SPECS, 'mixer'].map((s) => `${s}.spec.ts`),
      // Playwright matches the project name, the file name, the describes and the title, joined by spaces: of
      // `mixer.spec.ts`, only the render cost test.
      grepInvert: new RegExp(`(^|[ /])mixer\\.spec\\.ts (?!.*${PREVIEW_MIXER_TEST.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`),
      use: { baseURL: `http://localhost:${previewPort}`, storageState: onGrid(`http://localhost:${previewPort}`) },
    },
  ].filter((p) => wanted.includes(p.name)),
  webServer: [
    ...(wanted.includes('dev') ? [{ command: `pnpm exec vite dev --port ${devPort}`, url: `http://localhost:${devPort}/app`, timeout: 300_000 }] : []),
    // The e2e build also carries the test surfaces under /dev (the time grid's, AR-3); a production build has them empty.
    ...(wanted.includes('preview')
      ? [
          {
            command: `pnpm build && pnpm exec vite preview --port ${previewPort}`,
            url: `http://localhost:${previewPort}/app`,
            timeout: 300_000,
            env: { VITE_FAULK_DEV_ROUTES: '1' },
          },
        ]
      : []),
  ],
})
