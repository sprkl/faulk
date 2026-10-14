import { createHash } from 'node:crypto'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { devDependencies as dependencies } from './package.json' with { type: 'json' }
import adapter from '@sveltejs/adapter-static'
import { sveltekit } from '@sveltejs/kit/vite'
import type { Plugin } from 'vite'
import { defineConfig } from 'vitest/config'

/**
 * The landing page's animation module (`src/lib/landing/anim/index.ts`) is not imported by any
 * page: the prerendered pages stay plain HTML (`csr = false`) and only load it lazily, from an
 * inline bootstrap, when a sound object nears the viewport. This plugin bundles it as its own entry
 * of the client build, and exposes its URL to the server build (which prerenders the pages)
 * through the virtual module `virtual:landing-anim`.
 *
 * The server build runs before the client build, so the file name cannot come from the bundle: it
 * carries a hash of the module's sources and of its one dependency's version (OGL) instead,
 * computed up front. `anim.test.ts` checks that the module imports nothing else.
 */
function landingAnim(): Plugin {
  const virtualId = 'virtual:landing-anim'
  const resolvedId = '\0' + virtualId
  const dir = fileURLToPath(new URL('./src/lib/landing/anim/', import.meta.url))
  const hash = createHash('sha256')
  hash.update(String(dependencies.ogl))
  for (const file of readdirSync(dir, { recursive: true, encoding: 'utf8' }).sort()) {
    const path = join(dir, file)
    if (statSync(path).isFile()) hash.update(file).update(readFileSync(path))
  }
  const fileName = `landing/anim.${hash.digest('hex').slice(0, 8)}.js`
  let serve = false
  return {
    name: 'faulk-landing-anim',
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
      return `export const url = ${JSON.stringify(serve ? '/src/lib/landing/anim/index.ts' : '/' + fileName)}`
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
    landingAnim(),
  ],
  server: { port: 5173, strictPort: true },
  test: { include: ['src/**/*.test.ts'] },
})
