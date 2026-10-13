import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [svelte()],
  server: { port: 5173, strictPort: true },
  test: { include: ['src/**/*.test.ts'] },
})
