import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { describe, it } from 'node:test'
import { fileURLToPath } from 'node:url'
import { BASE, FILES, checkDay, parseArgs } from './check-urls.mjs'

const ROOT = dirname(dirname(dirname(fileURLToPath(import.meta.url))))
const TYPES = { 'track.mp3': 'audio/mpeg', 'track.mid': 'audio/midi', 'sheet.md': 'text/plain; charset=utf-8', 'project.faulkpack': 'application/octet-stream' }
const BYTES = new Uint8Array([1, 2, 3])

/** Runs the script without blocking this process (the test's own HTTP server must keep answering). */
const asyncSpawn = (script, args) =>
  new Promise((resolve) => {
    const child = spawn(process.execPath, [script, ...args])
    let out = ''
    child.stdout.on('data', (d) => (out += d))
    child.stderr.on('data', (d) => (out += d))
    child.on('close', (code) => resolve({ code, out }))
  })

/** A fake `fetch`: `over(name)` may change the answer of one file. */
const fakeFetch =
  (over = () => ({})) =>
  async (url) => {
    const name = url.split('/').pop()
    const a = { status: 200, type: TYPES[name], cors: '*', body: BYTES, ...over(name) }
    if (a.fail) throw new Error('network down')
    const headers = new Map([['content-type', a.type]])
    if (a.cors !== null) headers.set('access-control-allow-origin', a.cors)
    return {
      status: a.status,
      headers: { get: (k) => headers.get(k) ?? null },
      arrayBuffer: async () => a.body.buffer.slice(a.body.byteOffset, a.body.byteOffset + a.body.byteLength),
    }
  }
const run = (over, reference) => checkDay({ folder: '2026-01-01-x', fetchFn: fakeFetch(over), reference })

describe('check-urls', () => {
  it('uses the host the page uses', () => {
    const entries = readFileSync(join(ROOT, 'web/src/lib/journey/entries.ts'), 'utf8')
    assert.ok(entries.includes('const CDN = `https://raw.githubusercontent.com/${JOURNEY_REPO}/main/tracks`'))
    assert.equal(BASE, 'https://raw.githubusercontent.com/sprkl/faulk-journey/main/tracks')
  })

  it('passes four good files, checked against the reference bytes', async () => {
    const r = await run(undefined, async () => BYTES)
    assert.deepEqual(
      r.map((x) => x.file),
      FILES.map((f) => f.name),
    )
    assert.ok(r.every((x) => x.ok && x.sha256 !== null))
  })

  it('names each defect: status, type, CORS, bytes, network', async () => {
    const odd = { 'track.mid': { status: 403 }, 'track.mp3': { type: 'text/plain' }, 'sheet.md': { cors: null }, 'project.faulkpack': { fail: true } }
    const r = await run((n) => odd[n] ?? {}, async () => BYTES)
    const by = Object.fromEntries(r.map((x) => [x.file, x]))
    assert.match(by['track.mid'].problems.join(), /status 403/)
    assert.match(by['track.mp3'].problems.join(), /content-type "text\/plain"/)
    assert.match(by['sheet.md'].problems.join(), /CORS/)
    assert.match(by['project.faulkpack'].problems.join(), /unreachable: network down/)
    assert.ok(r.every((x) => !x.ok))
    const changed = await run(undefined, async () => new Uint8Array([9]))
    assert.ok(changed.every((x) => !x.ok && /bytes differ/.test(x.problems[0])))
  })

  it('checks status, type and CORS only when no reference is given', async () => {
    assert.ok((await run()).every((x) => x.ok))
  })

  it('runs as a command: reads --dir from disk, queries --base, exits 0 or 1', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'journey-'))
    for (const f of FILES) writeFileSync(join(dir, f.name), BYTES)
    const server = createServer((req, res) => {
      const name = req.url.split('/').pop()
      res.writeHead(200, { 'content-type': TYPES[name], 'access-control-allow-origin': '*' })
      res.end(Buffer.from(BYTES))
    })
    await new Promise((r) => server.listen(0, '127.0.0.1', r))
    const base = `http://127.0.0.1:${server.address().port}/tracks`
    const script = join(ROOT, 'scripts/journey/check-urls.mjs')
    const run1 = (...args) => spawnSync(process.execPath, [script, '2026-01-01-x', '--base', base, ...args], { encoding: 'utf8' })
    try {
      const good = await asyncSpawn(script, ['2026-01-01-x', '--base', base, '--dir', dir])
      assert.equal(good.code, 0, good.out)
      assert.match(good.out, /ok {3}project\.faulkpack/)
      writeFileSync(join(dir, 'track.mp3'), new Uint8Array([7, 7]))
      const stale = await asyncSpawn(script, ['2026-01-01-x', '--base', base, '--dir', dir])
      assert.equal(stale.code, 1)
      assert.match(stale.out, /FAIL track\.mp3: bytes differ/)
      const bad = run1('--dir')
      assert.equal(bad.status, 1)
    } finally {
      server.close()
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('refuses a bad command line', () => {
    assert.throws(() => parseArgs([]), /<date>-<slug>/)
    assert.throws(() => parseArgs(['../x']), /<date>-<slug>/)
    assert.throws(() => parseArgs(['2026-01-01-x', '--dir', 'a', '--commit', 'abc1234']), /not both/)
    assert.throws(() => parseArgs(['2026-01-01-x', '--commit', 'zz']), /hex/)
    assert.throws(() => parseArgs(['2026-01-01-x', '--bogus']), /unknown/)
    assert.equal(parseArgs(['2026-01-01-x', '--dir', 'd']).dir, 'd')
  })
})
