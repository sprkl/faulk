import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

/** Newest modification time of the files under a path (a `target` folder is skipped); a file's own time. */
export function newest(path: string): number {
  const stat = statSync(path)
  if (!stat.isDirectory()) return stat.mtimeMs
  let t = 0
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    if (entry.name === 'target') continue
    t = Math.max(t, newest(`${path}/${entry.name}`))
  }
  return t
}

/** The file next to the outputs that records the toolchain file's content they were built with. */
export const STAMP = 'toolchain.stamp'

/**
 * Whether the prototype's artefacts must be rebuilt (#873): one is missing, older than a source, or built with another
 * `rust-toolchain.toml` than today's. The stamp holds that file's content, so a new compiler or libc (Rust 1.100, #872) is seen
 * even when no mtime moves (a cached `build/`, an extracted archive).
 */
export function protoStale(outputs: string[], sources: string[], toolchain: string, stamp: string): boolean {
  if (outputs.some((o) => !existsSync(o))) return true
  if (!existsSync(stamp) || readFileSync(stamp, 'utf8') !== readFileSync(toolchain, 'utf8')) return true
  return Math.min(...outputs.map((o) => statSync(o).mtimeMs)) < Math.max(...sources.map(newest))
}

/** Records the toolchain file the outputs were just built with. */
export function writeStamp(toolchain: string, stamp: string): void {
  writeFileSync(stamp, readFileSync(toolchain, 'utf8'))
}

/** The inputs of the check for the prototype under `web/src/lib/engine/proto/`; the spec and its test share them. */
export function protoInputs() {
  const proto = fileURLToPath(new URL('../src/lib/engine/proto/', import.meta.url))
  const build = `${proto}build/`
  const crates = fileURLToPath(new URL('../../crates/', import.meta.url))
  return {
    outputs: [`${build}plain.wasm`, `${build}shared.wasm`],
    sources: [`${proto}crate`, `${crates}engine`, `${crates}engine-wasm`],
    toolchain: fileURLToPath(new URL('../../rust-toolchain.toml', import.meta.url)),
    stamp: `${build}${STAMP}`,
  }
}
