#!/usr/bin/env node
// Checks that the four files of a Journey day are served as the page needs them (#945): status 200,
// the right Content-Type, open CORS and the bytes of the committed file (SHA-256).
//
//   node scripts/journey/check-urls.mjs <date>-<slug> [--dir <local day folder>] [--commit <sha>] [--base <url>]
//
// The reference bytes come from `--dir` (the day's folder on disk) or from `--commit` (the same file
// pinned to that commit of the audio repository); with neither, only status, type and CORS are checked.
// Exit code 0 when every check passes, 1 otherwise.

import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

export const REPO = 'sprkl/faulk-journey'
/** The host the page reads from: keep in step with `web/src/lib/journey/entries.ts` (a test compares them). */
export const BASE = `https://raw.githubusercontent.com/${REPO}/main/tracks`
const COMMIT_BASE = (sha) => `https://raw.githubusercontent.com/${REPO}/${sha}/tracks`

/** The Content-Type each file must carry: `nosniff` makes the browser trust it. The sheet is read on GitHub, not here. */
export const FILES = [
  { name: 'track.mp3', type: /^audio\/(mpeg|mp3)$/ },
  { name: 'track.mid', type: /^audio\/(midi|mid|x-midi)$/ },
  { name: 'sheet.md', type: /^text\/(plain|markdown)$/ },
  { name: 'project.faulkpack', type: /^application\/octet-stream$/ },
]

const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex')

/**
 * Checks the four files of `folder`. `reference(file)` returns the expected bytes (or `null`); `fetchFn` is
 * `fetch`. Returns one `{ file, ok, problems, sha256 }` per file.
 */
export async function checkDay({ folder, base = BASE, fetchFn = fetch, reference = async () => null }) {
  const out = []
  for (const { name, type } of FILES) {
    const problems = []
    let digest = null
    try {
      const res = await fetchFn(`${base}/${folder}/${name}`, { headers: { Origin: 'https://faulk.app' }, cache: 'no-store' })
      if (res.status !== 200) problems.push(`status ${res.status}`)
      const ct = (res.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase()
      if (!type.test(ct)) problems.push(`content-type "${ct}"`)
      if (res.headers.get('access-control-allow-origin') !== '*') problems.push('no open CORS (access-control-allow-origin: *)')
      if (res.status === 200) {
        digest = sha256(new Uint8Array(await res.arrayBuffer()))
        const expected = await reference(name)
        if (expected !== null && sha256(expected) !== digest) problems.push('bytes differ from the reference file')
      }
    } catch (error) {
      problems.push(`unreachable: ${error instanceof Error ? error.message : String(error)}`)
    }
    out.push({ file: name, ok: problems.length === 0, problems, sha256: digest })
  }
  return out
}

export function parseArgs(argv) {
  const opts = { folder: null, dir: null, commit: null, base: BASE }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--dir' || a === '--commit' || a === '--base') {
      const v = argv[++i]
      if (v === undefined) throw new Error(`${a} needs a value`)
      opts[a.slice(2)] = v
    } else if (a.startsWith('--')) throw new Error(`unknown option ${a}`)
    else if (opts.folder === null) opts.folder = a
    else throw new Error('one day only')
  }
  if (opts.folder === null || !/^\d{4}-\d{2}-\d{2}-[a-z0-9]+(?:-[a-z0-9]+)*$/.test(opts.folder)) throw new Error('give the day as <date>-<slug>')
  if (opts.dir !== null && opts.commit !== null) throw new Error('--dir or --commit, not both')
  if (opts.commit !== null && !/^[0-9a-f]{7,40}$/.test(opts.commit)) throw new Error('--commit takes a hex commit id')
  return opts
}

async function main() {
  const opts = parseArgs(process.argv.slice(2))
  let reference = async () => null
  if (opts.dir !== null) reference = (name) => readFile(`${opts.dir}/${name}`)
  else if (opts.commit !== null) {
    reference = async (name) => {
      const res = await fetch(`${COMMIT_BASE(opts.commit)}/${opts.folder}/${name}`, { cache: 'no-store' })
      if (res.status !== 200) throw new Error(`reference ${name}: status ${res.status}`)
      return new Uint8Array(await res.arrayBuffer())
    }
  }
  const results = await checkDay({ folder: opts.folder, base: opts.base, reference })
  for (const r of results) console.log(`${r.ok ? 'ok  ' : 'FAIL'} ${r.file}${r.problems.length ? `: ${r.problems.join('; ')}` : ''}${r.sha256 ? ` (sha256 ${r.sha256.slice(0, 12)})` : ''}`)
  process.exitCode = results.every((r) => r.ok) ? 0 : 1
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  })
}
