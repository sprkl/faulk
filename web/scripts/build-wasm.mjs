// Builds the two WebAssembly modules of the DAW and copies them next to their loaders:
// the engine (crates/engine-wasm, for wasm32-wasip1-threads: one module, two instances on a
// shared memory, link arguments in .cargo/config.toml) and the page's project and commands
// (crates/project-wasm, for wasm32-unknown-unknown).
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../..', import.meta.url))
const lib = fileURLToPath(new URL('../src/lib/', import.meta.url))
const cargo = process.env.CARGO ?? 'cargo'

// Where cargo puts its output: `target/` of the checkout, or what CARGO_TARGET_DIR, CARGO_BUILD_TARGET_DIR
// or `build.target-dir` say (a build cache shared by several checkouts). Cargo itself answers.
function targetDirectory() {
  const json = execFileSync(cargo, ['metadata', '--no-deps', '--format-version', '1'], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
  return JSON.parse(json).target_directory
}
const targetDir = targetDirectory()

const builds = [
  { crate: 'engine-wasm', target: 'wasm32-wasip1-threads', file: 'engine_wasm.wasm', out: 'engine/wasm/engine.wasm' },
  { crate: 'project-wasm', target: 'wasm32-unknown-unknown', file: 'project_wasm.wasm', out: 'project/wasm/project.wasm' },
]

for (const { crate, target, file, out } of builds) {
  execFileSync(cargo, ['build', '-q', '-p', crate, '--release', '--target', target], { cwd: root, stdio: 'inherit' })
  mkdirSync(dirname(lib + out), { recursive: true })
  copyFileSync(join(targetDir, target, 'release', file), lib + out)
  console.log(`built src/lib/${out}`)
}
