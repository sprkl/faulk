import { defineConfig } from '@playwright/test'

/**
 * End-to-end tests in a real browser, against the dev server (`pnpm dev`) and the built site under
 * the static preview of a fresh build (`pnpm build && pnpm preview`), on their own ports (5180, 4180) so
 * that a server left running on 5173 or 4173 does not get in the way. The preview serves
 * the same headers as production (`vercel.json`, mirrored by `scripts/vercel-headers.ts`). The dev
 * server skips the wasm build of `pnpm dev`: `pnpm build` makes it, and two concurrent copies of
 * the same file could race.
 *
 * The browser is Playwright's Chromium; `FAULK_E2E_CHANNEL=chrome` or `msedge` runs the installed
 * Chrome or Edge instead. One worker: the helper test listens on the helper's fixed port. The
 * servers are always started here, never reused: a server already on those ports may come from
 * another checkout, and the run then fails on the busy port instead of testing other code.
 */
const channel = process.env.FAULK_E2E_CHANNEL || undefined
// The page and the stand-in helper of the helper test agree on this port (`tests/e2e/support.ts`).
if (process.env.FAULK_E2E_HELPER_PORT) process.env.VITE_FAULK_HELPER_PORT = process.env.FAULK_E2E_HELPER_PORT

export default defineConfig({
  testDir: 'tests/e2e',
  workers: 1,
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: process.env.CI ? [['list'], ['github']] : 'list',
  use: { channel, locale: 'en-US', trace: 'retain-on-failure' },
  projects: [
    { name: 'dev', use: { baseURL: 'http://localhost:5180' } },
    { name: 'preview', use: { baseURL: 'http://localhost:4180' } },
  ],
  webServer: [
    { command: 'pnpm exec vite dev --port 5180', url: 'http://localhost:5180/app', timeout: 300_000 },
    { command: 'pnpm build && pnpm exec vite preview --port 4180', url: 'http://localhost:4180/app', timeout: 300_000 },
  ],
})
