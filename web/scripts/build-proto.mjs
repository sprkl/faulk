// Builds the engine channel prototype (src/lib/engine/proto/crate, CT-3 #39) twice, on the
// pinned stable toolchain: for wasm32-unknown-unknown (way a, plain memory) and for
// wasm32-wasip1-threads (way b, shared memory, atomics, the second instance gets its own
// stack through the exported `__stack_pointer`). The e2e suite measures both.
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { withTargetLock } from './target-lock.mjs'
import { fileURLToPath } from 'node:url'

const crate = fileURLToPath(new URL('../src/lib/engine/proto/crate/', import.meta.url))
const dest = fileURLToPath(new URL('../src/lib/engine/proto/build/', import.meta.url))
const cargo = process.env.CARGO ?? 'cargo'

// The crate is outside the workspace: cargo says where its output goes (its own target/, or CARGO_TARGET_DIR).
const targetDir = JSON.parse(execFileSync(cargo, ['metadata', '--no-deps', '--format-version', '1'], { cwd: crate, encoding: 'utf8' })).target_directory

const builds = [
  { target: 'wasm32-unknown-unknown', out: 'plain.wasm', rustflags: '' },
  {
    target: 'wasm32-wasip1-threads',
    out: 'shared.wasm',
    rustflags: '-C link-arg=--export=__stack_pointer -C link-arg=--max-memory=4294967296',
  },
]

mkdirSync(dest, { recursive: true })
for (const { target, out, rustflags } of builds) {
  withTargetLock(targetDir, () => {
    execFileSync(cargo, ['build', '-q', '--release', '--target', target], {
      cwd: crate,
      stdio: 'inherit',
      env: { ...process.env, RUSTFLAGS: rustflags },
    })
    copyFileSync(join(targetDir, target, 'release', 'engine_proto.wasm'), dest + out)
  })
  console.log(`built src/lib/engine/proto/build/${out}`)
}
