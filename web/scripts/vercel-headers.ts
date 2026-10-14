/**
 * The response headers that production (Vercel) sets, applied to the local servers too.
 *
 * `vercel.json` is the single source of the headers: the dev server (`pnpm dev`) and the static
 * preview (`pnpm preview`) read its `headers` entries and set them on every response, so a check
 * made locally (cross-origin isolation) holds in production. Only site-wide entries (source
 * `/(.*)`, no `has`/`missing` condition) are supported: any other entry is refused when the config
 * loads, rather than matched differently from Vercel.
 */
import { readFileSync } from 'node:fs'
import type { IncomingMessage, ServerResponse } from 'node:http'
import type { Plugin } from 'vite'

/** The only source mirrored locally: every path. */
export const SITE_WIDE = '/(.*)'

export interface Header {
  key: string
  value: string
}

/** The headers of a parsed `vercel.json`; throws on an entry it cannot mirror. */
export function siteWideHeaders(config: unknown): Header[] {
  const entries = (config as { headers?: unknown }).headers
  if (entries === undefined) return []
  if (!Array.isArray(entries)) throw new Error('vercel.json: "headers" must be an array')
  return entries.flatMap((entry: unknown, i) => {
    const { source, headers, ...rest } = entry as Record<string, unknown>
    const where = `vercel.json: headers[${i}]`
    const extra = Object.keys(rest)
    if (extra.length > 0) throw new Error(`${where}: unsupported field(s) ${extra.join(', ')}`)
    if (source !== SITE_WIDE) {
      throw new Error(`${where}: only "${SITE_WIDE}" is mirrored locally, got ${JSON.stringify(source)}`)
    }
    if (
      !Array.isArray(headers) ||
      headers.length === 0 ||
      !headers.every((h) => typeof h?.key === 'string' && typeof h?.value === 'string')
    ) {
      throw new Error(`${where}: "headers" must be a non-empty list of { key, value } strings`)
    }
    return headers as Header[]
  })
}

/** Vite plugin: sets the `vercel.json` headers on every response of the dev and preview servers. */
export function vercelHeaders(file: URL): Plugin {
  const headers = siteWideHeaders(JSON.parse(readFileSync(file, 'utf8')))
  const middleware = (_req: IncomingMessage, res: ServerResponse, next: () => void) => {
    for (const { key, value } of headers) res.setHeader(key, value)
    next()
  }
  return {
    name: 'faulk-vercel-headers',
    // Before SvelteKit's own middlewares, which answer the page requests.
    enforce: 'pre',
    configureServer(server) {
      server.middlewares.use(middleware)
    },
    configurePreviewServer(server) {
      server.middlewares.use(middleware)
    },
  }
}
