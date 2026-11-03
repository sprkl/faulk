import { createHash } from 'node:crypto'
import { fileURLToPath } from 'node:url'

/**
 * The port of the end-to-end tests' stand-in helper (#505). The helper's own port, 47800, is the product's: an installed helper holds it,
 * and two checkouts running the suite at once would fight over it. So the suite takes a default port of its own, derived from the path of
 * the checkout's `web/` folder: two checkouts get two ports (up to a one in 190 collision, which a busy port says by name), and the
 * installed helper's port is never one of them. `FAULK_E2E_HELPER_PORT` still wins (`scripts/agent/verify.mjs` picks a free port that way).
 * The page is built to reach it (`VITE_FAULK_HELPER_PORT`, `playwright.config.ts`) and the specs listen on it (`tests/e2e/support.ts`).
 */

/** The first port of the range the default is drawn from: above the helper's 47800 and its neighbours. */
export const E2E_HELPER_PORT_FIRST = 47810
/** The size of that range: 47810 to 47999. */
export const E2E_HELPER_PORT_COUNT = 190

/** The `web/` folder of this checkout. */
const WEB_DIR = fileURLToPath(new URL('..', import.meta.url))

/** The default port of the checkout whose `web/` folder is `dir`: the same path always gives the same port, whatever its separators and case. */
export function checkoutHelperPort(dir: string): number {
  const key = dir.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase()
  const n = createHash('sha256').update(key).digest().readUInt32BE(0)
  return E2E_HELPER_PORT_FIRST + (n % E2E_HELPER_PORT_COUNT)
}

/** The port the suite uses: `FAULK_E2E_HELPER_PORT` when set (a whole number from 1 to 65535, or it throws), else this checkout's. */
export function e2eHelperPort(env: Record<string, string | undefined> = process.env, dir: string = WEB_DIR): number {
  const set = env.FAULK_E2E_HELPER_PORT?.trim()
  if (!set) return checkoutHelperPort(dir)
  const port = Number(set)
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error(`FAULK_E2E_HELPER_PORT: "${set}" is not a port (1 to 65535)`)
  return port
}
