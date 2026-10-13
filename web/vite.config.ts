import adapter from '@sveltejs/adapter-static'
import { sveltekit } from '@sveltejs/kit/vite'
import { defineConfig } from 'vitest/config'

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
  ],
  server: { port: 5173, strictPort: true },
  test: { include: ['src/**/*.test.ts'] },
})
