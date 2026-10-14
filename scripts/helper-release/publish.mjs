#!/usr/bin/env node
// Command line of the helper's Blob publication (logic in publish-core.mjs).
//
//   node scripts/helper-release/publish.mjs publish --dir <velopack output> --os win --version <X.Y.Z[-pre]> [--dry-run]
//   node scripts/helper-release/publish.mjs previous --os win --out <dir>
//
// `publish` uploads one release; `previous` downloads the newest stable full package into
// <dir>, so that `vpk pack` builds a delta from it. Both read the store with the token in
// BLOB_READ_WRITE_TOKEN, except `publish --dry-run`, which runs against an empty in-memory
// store and prints what it would write. `--vercel-json` (default web/vercel.json) gives the
// Blob origin get.faulk.app redirects to; a token for another store is refused.
// `@vercel/blob` is loaded only for a real run: `npm ci` in this folder first.

import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { blobOriginFromVercelJson, memoryStore, previousFull, publish } from './publish-core.mjs'

const here = import.meta.dirname

/** The real store: the public Blob store of BLOB_READ_WRITE_TOKEN. */
async function blobStore() {
  if (!process.env.BLOB_READ_WRITE_TOKEN) {
    throw new Error('BLOB_READ_WRITE_TOKEN is not set (GitHub secret HELPER_RELEASES_READ_WRITE_TOKEN)')
  }
  const blob = await import('@vercel/blob')
  const head = async (path) => {
    try {
      return await blob.head(path)
    } catch (e) {
      if (e instanceof blob.BlobNotFoundError) return null
      throw e
    }
  }
  return {
    head,
    async read(path) {
      // `head` asks the Blob API, which is current. The public URL goes through the cache,
      // which may serve an overwritten file for up to 60 s: the etag, from the API, makes the
      // URL of each version distinct so the channel guard never reads an older feed.
      const meta = await head(path)
      if (!meta) return null
      const url = new URL(meta.url)
      url.searchParams.set('v', meta.etag ?? Date.now().toString())
      const res = await fetch(url, { cache: 'no-store' })
      if (!res.ok) throw new Error(`GET ${meta.url}: HTTP ${res.status}`)
      return Buffer.from(await res.arrayBuffer())
    },
    async put(path, bytes, opts) {
      return blob.put(path, bytes, {
        access: 'public',
        addRandomSuffix: false,
        // Multipart parts are at least 5 MB; small files go in one request.
        multipart: bytes.length > 16 * 1024 * 1024,
        ...opts,
      })
    },
  }
}

function readDir(dir) {
  const files = new Map()
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name)
    if (statSync(path).isFile()) files.set(name, readFileSync(path))
  }
  return files
}

async function main() {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      dir: { type: 'string' },
      out: { type: 'string' },
      os: { type: 'string', default: 'win' },
      version: { type: 'string' },
      'dry-run': { type: 'boolean', default: false },
      'vercel-json': { type: 'string', default: resolve(here, '../../web/vercel.json') },
    },
  })
  const origin = blobOriginFromVercelJson(JSON.parse(readFileSync(values['vercel-json'], 'utf8')))
  const log = (line) => console.log(line)
  switch (positionals[0]) {
    case 'publish': {
      if (!values.dir || !values.version) throw new Error('publish needs --dir and --version')
      const dry = values['dry-run']
      const store = dry ? memoryStore(origin) : await blobStore()
      log(`${dry ? '[dry-run] ' : ''}publishing ${values.version} (${values.os}) to ${origin}`)
      const result = await publish({
        files: readDir(values.dir),
        version: values.version,
        os: values.os,
        store,
        expectedOrigin: origin,
        log,
      })
      log(
        result.feedWritten
          ? `done: the ${result.channel} feed offers ${values.version}`
          : `done: packages only, the ${result.channel} feed was not changed`,
      )
      break
    }
    case 'previous': {
      if (!values.out) throw new Error('previous needs --out')
      const found = await previousFull({ store: await blobStore(), os: values.os, log })
      if (found) writeFileSync(join(values.out, found.name), found.bytes)
      break
    }
    default:
      throw new Error('usage: publish.mjs publish|previous [options] (see the header of this file)')
  }
}

main().catch((e) => {
  console.error(`::error::${e.message}`)
  process.exit(1)
})
