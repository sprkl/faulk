import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, it } from 'node:test'
import {
  blobOriginFromVercelJson,
  SITE_ORIGIN,
  channelOf,
  channelPath,
  classify,
  compareVersions,
  IMMUTABLE_MAX_AGE,
  memoryStore,
  MUTABLE_MAX_AGE,
  packagePath,
  previousFull,
  publish,
  sha256,
  tokenMatchesOrigin,
} from './publish-core.mjs'

const ORIGIN = 'https://abc123.public.blob.vercel-storage.com'

function asset(version, type, bytes) {
  return {
    PackageId: 'FaulkHelper',
    Version: version,
    Type: type,
    FileName: `FaulkHelper-${version}-${type.toLowerCase()}.nupkg`,
    SHA256: sha256(bytes).toUpperCase(),
    Size: bytes.length,
  }
}

function feed(...assets) {
  // vpk writes its feeds with a BOM.
  return Buffer.from('﻿' + JSON.stringify({ Assets: assets }))
}

/** The Velopack output of `version`, with a delta from `previous` when given. */
function output(version, { previous, content = `full ${version}` } = {}) {
  const full = Buffer.from(content)
  const files = new Map([[`FaulkHelper-${version}-full.nupkg`, full]])
  const assets = [asset(version, 'Full', full)]
  if (previous) {
    const delta = Buffer.from(`delta ${previous}->${version}`)
    files.set(`FaulkHelper-${version}-delta.nupkg`, delta)
    assets.push(asset(version, 'Delta', delta), asset(previous, 'Full', Buffer.from(`full ${previous}`)))
  }
  files.set('FaulkHelper-win-Setup.exe', Buffer.from(`setup ${version}`))
  files.set('FaulkHelper-win-Portable.zip', Buffer.from(`portable ${version}`))
  files.set('assets.win.json', Buffer.from('[]'))
  files.set('RELEASES', Buffer.from(`sha FaulkHelper-${version}-full.nupkg 1`))
  files.set('releases.win.json', feed(...assets))
  return files
}

const puts = (store) => store.ops.filter((op) => op[0] === 'put').map((op) => op[1])

async function run(store, version, files = output(version), origin = ORIGIN) {
  return publish({ files, version, os: 'win', store, expectedOrigin: origin })
}

describe('versions', () => {
  it('orders by semantic version precedence', () => {
    const sorted = ['1.0.0', '0.10.0', '0.2.0', '0.2.0-beta.2', '0.2.0-beta.10', '0.2.0-alpha', '0.2.0-beta']
    sorted.sort(compareVersions)
    assert.deepEqual(sorted, ['0.2.0-alpha', '0.2.0-beta', '0.2.0-beta.2', '0.2.0-beta.10', '0.2.0', '0.10.0', '1.0.0'])
    assert.equal(compareVersions('0.1.0', '0.1.0'), 0)
    assert.throws(() => compareVersions('0.1', '0.1.0'))
  })

  it('puts a pre-release on beta, the rest on stable', () => {
    assert.equal(channelOf('0.1.0'), 'stable')
    assert.equal(channelOf('0.2.0-beta.1'), 'beta')
    assert.throws(() => channelOf('v0.1.0'))
  })
})

describe('classify', () => {
  it('sorts packages, downloads, index files and the feed', () => {
    const parts = classify([...output('0.2.0', { previous: '0.1.0' }).keys()], { os: 'win' })
    assert.deepEqual(parts.packages, ['FaulkHelper-0.2.0-delta.nupkg', 'FaulkHelper-0.2.0-full.nupkg'])
    assert.deepEqual(parts.downloads, ['FaulkHelper-win-Portable.zip', 'FaulkHelper-win-Setup.exe'])
    assert.deepEqual(parts.index, ['RELEASES', 'assets.win.json'])
    assert.equal(parts.feed, 'releases.win.json')
  })

  it('refuses an unknown file and an output without feed', () => {
    assert.throws(() => classify(['releases.win.json', 'notes.txt'], { os: 'win' }), /unexpected file notes.txt/)
    assert.throws(() => classify(['FaulkHelper-0.1.0-full.nupkg'], { os: 'win' }), /no releases.win.json/)
    assert.throws(() => classify(['../x'], { os: 'win' }), /unexpected file name/)
  })
})

describe('publish', () => {
  it('writes the packages first and the feed last, with the cache rules', async () => {
    const store = memoryStore(ORIGIN, { seed: new Map([['helper/win/packages/FaulkHelper-0.1.0-full.nupkg', Buffer.from('full 0.1.0')]]) })
    const result = await run(store, '0.2.0', output('0.2.0', { previous: '0.1.0' }))
    assert.deepEqual(result, { channel: 'stable', feedWritten: true })
    assert.deepEqual(puts(store), [
      'helper/win/packages/FaulkHelper-0.2.0-delta.nupkg',
      'helper/win/packages/FaulkHelper-0.2.0-full.nupkg',
      'helper/win/stable/FaulkHelper-win-Portable.zip',
      'helper/win/stable/FaulkHelper-win-Setup.exe',
      'helper/win/stable/RELEASES',
      'helper/win/stable/assets.win.json',
      'helper/win/stable/releases.win.json',
    ])
    for (const [op, path, opts] of store.ops) {
      if (op !== 'put') continue
      if (path.includes('/packages/')) assert.deepEqual([opts.allowOverwrite, opts.cacheControlMaxAge], [false, IMMUTABLE_MAX_AGE])
      else assert.deepEqual([opts.allowOverwrite, opts.cacheControlMaxAge], [true, MUTABLE_MAX_AGE])
    }
    assert.equal(store.blobs.get('helper/win/stable/releases.win.json').toString().includes('0.2.0'), true)
  })

  it('publishes a pre-release on the beta channel only', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.3.0-beta.1')
    assert.ok(store.blobs.has('helper/win/beta/releases.win.json'))
    assert.ok(![...store.blobs.keys()].some((k) => k.includes('/stable/')))
  })

  it('does not write the feed when it cites a package the store lacks', async () => {
    const store = memoryStore(ORIGIN) // 0.1.0-full, cited by the feed, is not there
    await assert.rejects(run(store, '0.2.0', output('0.2.0', { previous: '0.1.0' })), /cites FaulkHelper-0.1.0-full.nupkg/)
    assert.ok(!puts(store).some((p) => p.includes('/stable/')), 'nothing mutable written')
  })

  it('skips a package already published with the same content (re-run of the release job)', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.2.0')
    const before = puts(store).length
    await run(store, '0.2.0')
    const again = puts(store).slice(before)
    assert.ok(!again.some((p) => p.includes('/packages/')), 'packages not written again')
    assert.ok(again.includes('helper/win/stable/releases.win.json'), 'equal version: feed rewritten')
  })

  it('refuses to replace a published package with other content', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.2.0')
    const rebuilt = output('0.2.0', { content: 'another build of 0.2.0' })
    const before = store.ops.length
    await assert.rejects(run(store, '0.2.0', rebuilt), /already published with other content/)
    assert.ok(!store.ops.slice(before).some((op) => op[0] === 'put'), 'nothing written')
    assert.equal(store.blobs.get('helper/win/packages/FaulkHelper-0.2.0-full.nupkg').toString(), 'full 0.2.0')
  })

  it('never moves a channel backwards: an older version uploads its packages only', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.3.0')
    const before = puts(store).length
    const result = await run(store, '0.2.0')
    assert.deepEqual(result, { channel: 'stable', feedWritten: false })
    assert.deepEqual(puts(store).slice(before), ['helper/win/packages/FaulkHelper-0.2.0-full.nupkg'])
    assert.match(store.blobs.get('helper/win/stable/releases.win.json').toString(), /0\.3\.0/)
    assert.equal(store.blobs.get('helper/win/stable/FaulkHelper-win-Setup.exe').toString(), 'setup 0.3.0')
  })

  it('moves a channel forward', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.2.0')
    await run(store, '0.3.0', output('0.3.0', { previous: '0.2.0', content: 'full 0.3.0' }))
    assert.equal(store.blobs.get('helper/win/stable/FaulkHelper-win-Setup.exe').toString(), 'setup 0.3.0')
  })

  it('refuses a store that get.faulk.app does not redirect to, before anything mutable', async () => {
    const store = memoryStore('https://other.public.blob.vercel-storage.com')
    await assert.rejects(run(store, '0.2.0'), /get.faulk.app redirects to https:\/\/abc123/)
    assert.ok(!puts(store).some((p) => !p.includes('/packages/')))
  })

  it('refuses a feed without the full package of its version, or of another package id', async () => {
    const files = output('0.2.0')
    await assert.rejects(run(memoryStore(ORIGIN), '0.2.1', files), /does not list the full package of 0.2.1/)
    files.delete('FaulkHelper-0.2.0-full.nupkg')
    await assert.rejects(run(memoryStore(ORIGIN), '0.2.0', files), /or it is missing/)
    const other = output('0.2.0')
    const json = JSON.parse(other.get('releases.win.json').toString().slice(1))
    json.Assets.push({ ...json.Assets[0], PackageId: 'Other' })
    other.set('releases.win.json', Buffer.from(JSON.stringify(json)))
    await assert.rejects(run(memoryStore(ORIGIN), '0.2.0', other), /another package id/)
  })

  it('refuses a stray package: not of this version, or not cited by the feed', async () => {
    const withBase = output('0.2.0', { previous: '0.1.0' })
    withBase.set('FaulkHelper-0.1.0-full.nupkg', Buffer.from('full 0.1.0'))
    const store = memoryStore(ORIGIN)
    await assert.rejects(run(store, '0.2.0', withBase), /FaulkHelper-0.1.0-full.nupkg is not a package of 0.2.0/)
    const extra = output('0.2.0')
    extra.set('FaulkHelper-0.2.0-delta.nupkg', Buffer.from('not cited'))
    await assert.rejects(run(store, '0.2.0', extra), /not a package of 0.2.0 cited/)
    assert.equal(puts(store).length, 0, 'nothing written')
  })

  it('publishes Windows only: other systems would have unreachable packages', async () => {
    const store = memoryStore(ORIGIN)
    await assert.rejects(publish({ files: output('0.2.0'), version: '0.2.0', os: 'linux', store, expectedOrigin: ORIGIN }), /linux is not published yet/)
    await assert.rejects(publish({ files: output('0.2.0'), version: '0.2.0', os: 'beos', store, expectedOrigin: ORIGIN }), /unknown OS channel/)
    assert.equal(puts(store).length, 0)
  })

  it('refuses a feed that walks paths', async () => {
    const files = output('0.2.0')
    const json = JSON.parse(files.get('releases.win.json').toString().slice(1))
    json.Assets[0].FileName = '../../x.nupkg'
    files.set('releases.win.json', Buffer.from(JSON.stringify(json)))
    await assert.rejects(run(memoryStore(ORIGIN), '0.2.0', files), /unsafe file name/)
  })
})

describe('previousFull', () => {
  it('returns nothing when the store has no stable feed', async () => {
    const lines = []
    assert.equal(await previousFull({ store: memoryStore(ORIGIN), os: 'win', below: '0.2.0', log: (l) => lines.push(l) }), null)
    assert.match(lines[0], /no delta package/)
  })

  it('downloads the newest full package of the stable feed', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.1.0')
    await run(store, '0.2.0', output('0.2.0', { previous: '0.1.0' }))
    const found = await previousFull({ store, os: 'win', below: '0.3.0' })
    assert.equal(found.name, 'FaulkHelper-0.2.0-full.nupkg')
    assert.equal(found.bytes.toString(), 'full 0.2.0')
  })

  it('never takes the version being built, nor a newer one', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.1.0')
    await run(store, '0.2.0', output('0.2.0', { previous: '0.1.0' }))
    // A hotfix 0.1.1 built after 0.2.0: its base is 0.1.0.
    assert.equal((await previousFull({ store, os: 'win', below: '0.1.1' })).name, 'FaulkHelper-0.1.0-full.nupkg')
    // A re-run of 0.2.0: its own package is not its base.
    assert.equal((await previousFull({ store, os: 'win', below: '0.2.0' })).name, 'FaulkHelper-0.1.0-full.nupkg')
    // Older than everything published: no delta.
    const lines = []
    assert.equal(await previousFull({ store, os: 'win', below: '0.0.9', log: (l) => lines.push(l) }), null)
    assert.match(lines[0], /older than 0.0.9/)
    await assert.rejects(previousFull({ store, os: 'win' }), /needs the version being built/)
  })

  it('refuses a package that does not match the feed', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.1.0')
    store.blobs.set('helper/win/packages/FaulkHelper-0.1.0-full.nupkg', Buffer.from('tampered'))
    await assert.rejects(previousFull({ store, os: 'win', below: '0.2.0' }), /does not match the SHA-256/)
  })

  it('refuses a feed that cites a missing package', async () => {
    const store = memoryStore(ORIGIN)
    await run(store, '0.1.0')
    store.blobs.delete('helper/win/packages/FaulkHelper-0.1.0-full.nupkg')
    await assert.rejects(previousFull({ store, os: 'win', below: '0.2.0' }), /not in the store/)
  })
})

describe('tokenMatchesOrigin', () => {
  it('accepts a token of the store, whatever the case of its id', () => {
    assert.ok(tokenMatchesOrigin('vercel_blob_rw_AbC123_secretPart', ORIGIN))
  })

  it('refuses a token of another store, or anything that is not a read-write token', () => {
    assert.ok(!tokenMatchesOrigin('vercel_blob_rw_Other1_secret', ORIGIN))
    assert.ok(!tokenMatchesOrigin('vercel_blob_ro_AbC123_secret', ORIGIN))
    assert.ok(!tokenMatchesOrigin('AbC123', ORIGIN))
    assert.ok(!tokenMatchesOrigin('', ORIGIN))
    assert.ok(!tokenMatchesOrigin('vercel_blob_rw__secret', 'https://.public.blob.vercel-storage.com'))
  })
})

describe('web/vercel.json', () => {
  it('redirects get.faulk.app to one public Blob origin', () => {
    const json = JSON.parse(readFileSync(resolve(import.meta.dirname, '../../web/vercel.json'), 'utf8'))
    assert.match(blobOriginFromVercelJson(json), /^https:\/\/[a-z0-9]+\.public\.blob\.vercel-storage\.com$/)
  })

  it('lets a redirect lead to the public site, and only to it', () => {
    const rule = (destination) => ({ has: [{ type: 'host', value: 'get.faulk.app' }], destination })
    assert.equal(blobOriginFromVercelJson({ redirects: [rule(`${ORIGIN}/a`), rule(`${SITE_ORIGIN}/`)] }), ORIGIN)
    assert.throws(() => blobOriginFromVercelJson({ redirects: [rule(`${SITE_ORIGIN}/`)] }), /0 origins/)
    assert.throws(() => blobOriginFromVercelJson({ redirects: [rule(`${ORIGIN}/a`), rule('https://www.faulk.app.evil.io/')] }), /2 origins/)
  })

  it('refuses several origins, or one that is not a public Blob store', () => {
    const rule = (destination) => ({ has: [{ type: 'host', value: 'get.faulk.app' }], destination })
    assert.throws(
      () => blobOriginFromVercelJson({ redirects: [rule(`${ORIGIN}/a`), rule('https://x.public.blob.vercel-storage.com/b')] }),
      /2 origins/,
    )
    assert.throws(() => blobOriginFromVercelJson({ redirects: [rule('https://github.com/x')] }), /not a public Vercel Blob store/)
    assert.throws(() => blobOriginFromVercelJson({ redirects: [] }), /0 origins/)
  })
})

/**
 * Applies the get.faulk.app redirects of web/vercel.json, translating the subset of
 * path-to-regexp they use (literal text with backslash escapes, `:name(regex)` parameters),
 * like the Rust test of the updater. The route compiler of Vercel itself
 * (`@vercel/routing-utils`) gave the same destinations when this was written.
 */
function redirect(json, path) {
  for (const rule of json.redirects) {
    const names = []
    let pattern = '^'
    let rest = rule.source
    while (rest.length) {
      const i = rest.indexOf(':')
      const literal = i < 0 ? rest : rest.slice(0, i)
      pattern += literal.replace(/\\(.)/g, '$1').replace(/[.*+?^${}()|[\]\\/]/g, '\\$&')
      if (i < 0) break
      const open = rest.indexOf('(', i)
      names.push(rest.slice(i + 1, open))
      let depth = 0
      let end = open
      for (; end < rest.length; end++) {
        if (rest[end] === '(') depth++
        if (rest[end] === ')' && --depth === 0) break
      }
      pattern += `(${rest.slice(open + 1, end)})`
      rest = rest.slice(end + 1)
    }
    const m = new RegExp(pattern + '$').exec(path)
    if (!m) continue
    let dest = rule.destination
    const order = names.map((n, k) => [n, m[k + 1]]).sort((a, b) => b[0].length - a[0].length)
    for (const [n, v] of order) dest = dest.replaceAll(`:${n}`, v)
    return dest
  }
  throw new Error(`no redirect for ${path}`)
}

describe('the URLs the helper reads land where the script writes', () => {
  const json = JSON.parse(readFileSync(resolve(import.meta.dirname, '../../web/vercel.json'), 'utf8'))
  const origin = blobOriginFromVercelJson(json)

  it('feeds, by channel and OS', () => {
    for (const os of ['win', 'linux', 'osx']) {
      for (const channel of ['stable', 'beta']) {
        const name = `releases.${os}.json`
        assert.equal(redirect(json, `/helper/${channel}/${name}`), `${origin}/${channelPath(os, channel, name)}`)
      }
    }
  })

  it('packages, full and delta, stable and pre-release', () => {
    for (const file of ['FaulkHelper-0.2.0-full.nupkg', 'FaulkHelper-0.2.0-delta.nupkg', 'FaulkHelper-0.3.0-beta.1-full.nupkg']) {
      assert.equal(redirect(json, `/helper/packages/${file}`), `${origin}/${packagePath('win', file)}`)
    }
  })

  it('the download links: the stable Windows installer', () => {
    for (const path of ['/helper/windows', '/helper/win/stable']) {
      assert.equal(redirect(json, path), `${origin}/${channelPath('win', 'stable', 'FaulkHelper-win-Setup.exe')}`)
    }
  })

  it('the root and any other path: the public site, never an installer (no macOS build exists yet, #587)', () => {
    for (const path of ['/', '/anything', '/helper/osx/stable', '/helper']) {
      assert.equal(redirect(json, path), `${SITE_ORIGIN}/`)
    }
  })

  it('the script publishes the installer under that name', () => {
    const parts = classify([...output('0.2.0').keys()], { os: 'win' })
    assert.ok(parts.downloads.includes('FaulkHelper-win-Setup.exe'))
  })
})
