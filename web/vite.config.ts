import { createHash } from 'node:crypto'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { devDependencies as dependencies } from './package.json' with { type: 'json' }
import adapter from '@sveltejs/adapter-static'
import { sveltekit } from '@sveltejs/kit/vite'
import type { Plugin } from 'vite'
import { defineConfig } from 'vitest/config'
import { vercelHeaders } from './scripts/vercel-headers.ts'

/**
 * A module that no page imports: the prerendered pages stay plain HTML (`csr = false`) and only
 * load it lazily, from an inline bootstrap. This plugin bundles it as its own entry of the client
 * build (`<folder>/index.ts`), and exposes its URL to the server build (which prerenders the pages)
 * through the virtual module `virtual:<name>`.
 *
 * The server build runs before the client build, so the file name cannot come from the bundle: it
 * carries a hash of the folder's sources and of its dependencies' versions instead, computed up
 * front. Each module's tests check that it imports nothing else (`anim.test.ts`, `live.test.ts`).
 *
 * - `landing-anim`: the landing page's sound objects (`src/lib/landing/anim/`, with OGL), loaded
 *   when one nears the viewport;
 * - `journey-live`: the Journey page's players and their visualiser (`src/lib/journey/live/`).
 */
function lazyModule(name: string, folder: string, fileStem: string, versions: string[]): Plugin {
  const virtualId = `virtual:${name}`
  const resolvedId = '\0' + virtualId
  const dir = fileURLToPath(new URL(`./src/lib/${folder}/`, import.meta.url))
  const hash = createHash('sha256')
  for (const version of versions) hash.update(version)
  for (const file of readdirSync(dir, { recursive: true, encoding: 'utf8' }).sort()) {
    const path = join(dir, file)
    if (statSync(path).isFile()) hash.update(file).update(readFileSync(path))
  }
  const fileName = `${fileStem}.${hash.digest('hex').slice(0, 8)}.js`
  let serve = false
  return {
    name: `faulk-${name}`,
    configResolved(config) {
      serve = config.command === 'serve'
    },
    resolveId(id) {
      return id === virtualId ? resolvedId : undefined
    },
    buildStart() {
      if (serve || this.environment.name !== 'client') return
      this.emitFile({ type: 'chunk', id: join(dir, 'index.ts'), fileName })
    },
    load(id) {
      if (id !== resolvedId) return
      return `export const url = ${JSON.stringify(serve ? `/src/lib/${folder}/index.ts` : '/' + fileName)}`
    },
  }
}

export default defineConfig({
  plugins: [
    sveltekit({
      compilerOptions: {
        // Runes everywhere except in dependencies.
        runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true),
      },
      // Every page is static: the landing page is prerendered HTML, `/app` an empty client-side shell.
      adapter: adapter({ pages: 'dist', assets: 'dist', strict: true }),
    }),
    lazyModule('landing-anim', 'landing/anim', 'landing/anim', [String(dependencies.ogl)]),
    lazyModule('journey-live', 'journey/live', 'journey/live', []),
    // Production's response headers (cross-origin isolation) on the local servers too.
    vercelHeaders(new URL('./vercel.json', import.meta.url)),
  ],
  server: { port: 5173, strictPort: true },
  preview: { port: 4173, strictPort: true },
  test: {
    projects: [
      // The tests of logic and of server-rendered views.
      { extends: true, test: { name: 'unit', include: ['src/**/*.test.ts', 'scripts/**/*.test.ts'], exclude: ['**/*.dom.test.ts', '**/node_modules/**'] } },
      // The tests that mount a component in a DOM and drive it with events: Svelte must resolve to its browser build.
      { extends: true, resolve: { conditions: ['browser'] }, test: { name: 'dom', include: ['src/**/*.dom.test.ts'], environment: 'jsdom' } },
    ],
  },
})
