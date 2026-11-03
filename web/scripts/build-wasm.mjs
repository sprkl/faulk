// Builds the three WebAssembly modules of the DAW and copies them next to their loaders:
// the engine (crates/engine-wasm, for wasm32-wasip1-threads: one module, two instances on a
// shared memory, link arguments in .cargo/config.toml), the page's project and commands
// (crates/project-wasm, for wasm32-unknown-unknown) and the audio import pipeline
// (crates/audio-import-wasm, for wasm32-unknown-unknown, run in the project store's Worker).
import { execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { withTargetLock } from './target-lock.mjs'
import { stripNames } from './wasm-names.mjs'

const root = fileURLToPath(new URL('../..', import.meta.url))
const lib = fileURLToPath(new URL('../src/lib/', import.meta.url))
const cargo = process.env.CARGO ?? 'cargo'
// `--strip-names` (the production build, `pnpm build`): the copies next to the loaders lose their "name" section, the
// function names of stack traces (#248). `pnpm dev`, `pnpm test` and `pnpm build:wasm` keep them, so an error in
// development names the function. The cargo output is the same either way: only the copy changes, and `postbuild`
// copies the named modules back, so the dev server and the e2e specs that follow a build still read them.
const strip = process.argv.includes('--strip-names')

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
  // The audio import pipeline (AU-1), loaded by the project store's Worker on the first import.
  { crate: 'audio-import-wasm', target: 'wasm32-unknown-unknown', file: 'audio_import_wasm.wasm', out: 'audio/import/wasm/import.wasm' },
]

for (const { crate, target, file, out } of builds) {
  // Build and copy under one lock: a checkout that shares the target directory must not replace the artifact in between.
  withTargetLock(targetDir, () => {
    execFileSync(cargo, ['build', '-q', '-p', crate, '--release', '--target', target], { cwd: root, stdio: 'inherit' })
    mkdirSync(dirname(lib + out), { recursive: true })
    const built = readFileSync(join(targetDir, target, 'release', file))
    writeFileSync(lib + out, strip ? stripNames(built) : built)
  })
  console.log(`built src/lib/${out}${strip ? ' (names stripped)' : ''}`)
}
