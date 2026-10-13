// Builds the engine as WebAssembly and copies it next to the worklet.
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../..', import.meta.url))
const dest = fileURLToPath(new URL('../src/lib/engine/wasm/', import.meta.url))
const cargo = process.env.CARGO ?? 'cargo'

execFileSync(
  cargo,
  ['build', '-q', '-p', 'engine-wasm', '--release', '--target', 'wasm32-unknown-unknown'],
  { cwd: root, stdio: 'inherit' },
)
mkdirSync(dest, { recursive: true })
copyFileSync(root + 'target/wasm32-unknown-unknown/release/engine_wasm.wasm', dest + 'engine.wasm')
console.log('built src/lib/engine/wasm/engine.wasm')
