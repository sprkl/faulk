// Publishes the Velopack output of a helper release to the Vercel Blob store that
// get.faulk.app redirects to. Pure logic over an injected store, so that every guard is
// tested without network or token (publish-core.test.mjs). The CLI is publish.mjs.
// See docs/architecture/04-distribution-helper.md.
//
// Layout in the store (pathnames, no random suffix):
//
//   helper/<os>/packages/<id>-<version>-full.nupkg | -delta.nupkg   immutable
//   helper/<os>/<channel>/<id>-<os>-Setup.exe | -Portable.zip       latest of the channel
//   helper/<os>/<channel>/assets.<os>.json, RELEASES                 latest of the channel
//   helper/<os>/<channel>/releases.<os>.json                         the feed, written last
//
// <os> is the Velopack OS channel (win, linux, osx); <channel> is stable, or beta for a
// pre-release version.

import { createHash } from 'node:crypto'

export const PACK_ID = 'FaulkHelper'
export const OS_CHANNELS = ['win', 'linux', 'osx']
/**
 * The systems that may be published. The helper asks for its packages at
 * `get.faulk.app/helper/packages/<file>`, without the system, and web/vercel.json sends that to
 * the Windows packages: macOS and Linux need a client change before they are published.
 */
export const PUBLISHED_OS = ['win']
/** Packages never change once published: cached for a year. */
export const IMMUTABLE_MAX_AGE = 31536000
/** The feed and the latest installer change at each release: Blob's minimum, 60 s. */
export const MUTABLE_MAX_AGE = 60

const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/

/** Parses `X.Y.Z[-pre]`; null when malformed. */
export function parseVersion(text) {
  const m = SEMVER.exec(String(text))
  if (!m) return null
  return { core: [Number(m[1]), Number(m[2]), Number(m[3])], pre: m[4] ? m[4].split('.') : [] }
}

/** Semantic version precedence: negative, zero or positive, like a sort comparator. */
export function compareVersions(a, b) {
  const x = parseVersion(a)
  const y = parseVersion(b)
  if (!x || !y) throw new Error(`cannot compare versions '${a}' and '${b}'`)
  for (let i = 0; i < 3; i++) if (x.core[i] !== y.core[i]) return x.core[i] - y.core[i]
  // A version without pre-release ranks above the same version with one.
  if (!x.pre.length || !y.pre.length) return y.pre.length - x.pre.length
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i++) {
    if (i >= x.pre.length) return -1
    if (i >= y.pre.length) return 1
    const [p, q] = [x.pre[i], y.pre[i]]
    const [pn, qn] = [/^\d+$/.test(p), /^\d+$/.test(q)]
    if (pn && qn && Number(p) !== Number(q)) return Number(p) - Number(q)
    if (pn !== qn) return pn ? -1 : 1
    if (!pn && p !== q) return p < q ? -1 : 1
  }
  return 0
}

/** The channel a version is published on. */
export function channelOf(version) {
  const v = parseVersion(version)
  if (!v) throw new Error(`version '${version}': expected X.Y.Z or X.Y.Z-suffix`)
  return v.pre.length ? 'beta' : 'stable'
}

/** A file name the feed may cite: one plain name, nothing that walks paths. */
export function isSafeFileName(name) {
  return (
    typeof name === 'string' &&
    name.length > 0 &&
    name.length <= 200 &&
    !name.includes('..') &&
    /^[0-9A-Za-z._-]+$/.test(name)
  )
}

export function packagePath(os, file) {
  return `helper/${os}/packages/${file}`
}

export function channelPath(os, channel, file) {
  return `helper/${os}/${channel}/${file}`
}

export function feedName(os) {
  return `releases.${os}.json`
}

/** Reads a Velopack feed (`releases.<os>.json`), which vpk writes with a BOM. */
export function parseFeed(bytes, label = 'feed') {
  let json
  try {
    json = JSON.parse(Buffer.from(bytes).toString('utf8').replace(/^﻿/, ''))
  } catch (e) {
    throw new Error(`${label}: not JSON (${e.message})`)
  }
  if (!json || !Array.isArray(json.Assets)) throw new Error(`${label}: no Assets array`)
  for (const a of json.Assets) {
    if (!isSafeFileName(a?.FileName)) throw new Error(`${label}: unsafe file name ${JSON.stringify(a?.FileName)}`)
    if (!parseVersion(a.Version)) throw new Error(`${label}: bad version ${JSON.stringify(a.Version)}`)
  }
  return json
}

/** Newest version listed in a feed, or null for an empty one. */
export function newestVersion(feed) {
  let newest = null
  for (const a of feed.Assets) if (newest === null || compareVersions(a.Version, newest) > 0) newest = a.Version
  return newest
}

export function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex')
}

/**
 * Sorts the files of one Velopack output into what goes where. Throws on a file it does
 * not know: an unexpected file is a packaging change to look at, not something to upload.
 */
export function classify(names, { os, packId = PACK_ID }) {
  const packages = []
  const downloads = []
  const index = []
  let feed = null
  for (const name of names) {
    if (!isSafeFileName(name)) throw new Error(`unexpected file name ${JSON.stringify(name)}`)
    if (name.startsWith(`${packId}-`) && /-(full|delta)\.nupkg$/.test(name)) packages.push(name)
    else if (name === `${packId}-${os}-Setup.exe` || name === `${packId}-${os}-Portable.zip`) downloads.push(name)
    else if (name === `assets.${os}.json` || name === 'RELEASES') index.push(name)
    else if (name === feedName(os)) feed = name
    else throw new Error(`unexpected file ${name} in the Velopack output`)
  }
  if (!feed) throw new Error(`no ${feedName(os)} in the Velopack output`)
  return { packages: packages.sort(), downloads: downloads.sort(), index: index.sort(), feed }
}

function contentType(name) {
  if (name.endsWith('.json')) return 'application/json'
  if (name.endsWith('.zip')) return 'application/zip'
  if (name === 'RELEASES') return 'text/plain; charset=utf-8'
  return 'application/octet-stream'
}

/**
 * Publishes one release.
 *
 * - `files`: Map of file name -> bytes (the Velopack output of this version);
 * - `store`: `{ head(path) -> {url} | null, read(path) -> bytes | null, put(path, bytes, opts) -> {url} }`;
 * - `expectedOrigin`: the Blob origin get.faulk.app redirects to; a store elsewhere is refused
 *   before anything mutable is written.
 *
 * Order: packages, then the check that every package the feed cites is in the store, then
 * the installer and portable, then `assets.<os>.json` and `RELEASES`, then the feed last. A
 * version older than the channel's current feed uploads its packages only.
 */
export async function publish({ files, version, os, store, expectedOrigin, packId = PACK_ID, log = () => {} }) {
  if (!OS_CHANNELS.includes(os)) throw new Error(`unknown OS channel ${os}`)
  if (!PUBLISHED_OS.includes(os)) {
    throw new Error(`${os} is not published yet: its packages would not be reachable (see PUBLISHED_OS)`)
  }
  const channel = channelOf(version)
  const parts = classify([...files.keys()], { os, packId })
  const feed = parseFeed(files.get(parts.feed), `local ${parts.feed}`)
  const own = feed.Assets.find((a) => a.Type === 'Full' && a.Version === version)
  if (!own || !files.has(own.FileName)) {
    throw new Error(`${parts.feed} does not list the full package of ${version}, or it is missing`)
  }
  for (const a of feed.Assets) {
    if (a.PackageId !== packId) throw new Error(`${parts.feed} lists another package id: ${a.PackageId}`)
  }
  // Only this version's packages are uploaded, and only those the feed cites: a stray package
  // would become an immutable, year-cached blob that a later real publication could not fix.
  // Packages of other versions the feed cites (the delta base) must already be in the store.
  for (const name of parts.packages) {
    const cited = feed.Assets.find((a) => a.FileName === name)
    if (!cited || cited.Version !== version) {
      throw new Error(`${name} is not a package of ${version} cited by ${parts.feed}: remove it from the folder`)
    }
  }
  const checkOrigin = (url, path) => {
    const origin = new URL(url).origin
    if (origin !== expectedOrigin) {
      throw new Error(
        `the store answers at ${origin} for ${path}, but get.faulk.app redirects to ${expectedOrigin}: ` +
          `fix web/vercel.json or the token (HELPER_RELEASES_READ_WRITE_TOKEN)`,
      )
    }
  }

  // Channel guard: a re-run of an older version must not move the channel backwards.
  const feedPath = channelPath(os, channel, parts.feed)
  const current = await store.read(feedPath)
  let mutable = true
  if (current) {
    const newest = newestVersion(parseFeed(current, `remote ${feedPath}`))
    if (newest && compareVersions(newest, version) > 0) {
      mutable = false
      log(`notice: the ${channel} feed already offers ${newest}, newer than ${version}: packages only`)
    }
  }

  // 1. Packages: written once, never replaced by different bytes.
  for (const name of parts.packages) {
    const path = packagePath(os, name)
    const bytes = files.get(name)
    const existing = await store.head(path)
    if (existing) {
      checkOrigin(existing.url, path)
      const remote = await store.read(path)
      if (remote && sha256(remote) === sha256(bytes)) {
        log(`skip ${path}: already published, same content`)
        continue
      }
      throw new Error(
        `${path} is already published with other content: a published version is never replaced. ` +
          `Publish a new version, or re-run only the release job (it reuses the built files).`,
      )
    }
    const put = await store.put(path, bytes, {
      allowOverwrite: false,
      cacheControlMaxAge: IMMUTABLE_MAX_AGE,
      contentType: contentType(name),
    })
    checkOrigin(put.url, path)
    log(`put ${path} (${bytes.length} bytes, immutable, max-age ${IMMUTABLE_MAX_AGE} s)`)
  }

  // 2. Every package the feed cites must be in the store before the feed points at it.
  for (const a of feed.Assets) {
    const path = packagePath(os, a.FileName)
    const found = await store.head(path)
    if (!found) throw new Error(`${parts.feed} cites ${a.FileName}, which is not in the store (${path}): feed not written`)
    checkOrigin(found.url, path)
  }

  if (!mutable) return { channel, feedWritten: false }

  // 3. Installer and portable, 4. the index files, 5. the feed, last.
  for (const name of [...parts.downloads, ...parts.index, parts.feed]) {
    const path = channelPath(os, channel, name)
    const put = await store.put(path, files.get(name), {
      allowOverwrite: true,
      cacheControlMaxAge: MUTABLE_MAX_AGE,
      contentType: contentType(name),
    })
    checkOrigin(put.url, path)
    log(`put ${path} (${files.get(name).length} bytes, overwritable, max-age ${MUTABLE_MAX_AGE} s)`)
  }
  return { channel, feedWritten: true }
}

/**
 * The delta base: the newest full package of the stable feed older than `below` (the version
 * being built), checked against the feed's SHA-256. Returns `{ name, bytes }`, or null when
 * there is none (first release, or a hotfix older than every published version).
 */
export async function previousFull({ store, os, below, log = () => {} }) {
  if (!parseVersion(below)) throw new Error(`previous needs the version being built, got '${below}'`)
  const feedPath = channelPath(os, 'stable', feedName(os))
  const bytes = await store.read(feedPath)
  if (!bytes) {
    log(`warning: no ${feedPath} in the store: this version gets no delta package`)
    return null
  }
  const fulls = parseFeed(bytes, feedPath).Assets.filter(
    (a) => a.Type === 'Full' && compareVersions(a.Version, below) < 0,
  )
  if (!fulls.length) {
    log(`warning: ${feedPath} lists no full package older than ${below}: this version gets no delta package`)
    return null
  }
  fulls.sort((a, b) => compareVersions(b.Version, a.Version))
  const newest = fulls[0]
  const path = packagePath(os, newest.FileName)
  const pkg = await store.read(path)
  if (!pkg) throw new Error(`${feedPath} cites ${newest.FileName}, which is not in the store`)
  if (typeof newest.SHA256 !== 'string' || sha256(pkg).toLowerCase() !== newest.SHA256.toLowerCase()) {
    throw new Error(`${path} does not match the SHA-256 of ${feedPath}`)
  }
  log(`delta base: ${newest.FileName} (${newest.Version})`)
  return { name: newest.FileName, bytes: pkg }
}

/**
 * The Blob origin of `web/vercel.json`: every get.faulk.app redirect must go to one https
 * origin of a public Blob store.
 */
export function blobOriginFromVercelJson(json) {
  const origins = new Set()
  for (const r of json.redirects ?? []) {
    const host = (r.has ?? []).find((h) => h.type === 'host')?.value
    if (host !== 'get.faulk.app') continue
    origins.add(new URL(r.destination).origin)
  }
  if (origins.size !== 1) throw new Error(`get.faulk.app redirects to ${origins.size} origins, expected one`)
  const [origin] = origins
  if (!/^https:\/\/[a-z0-9]+\.public\.blob\.vercel-storage\.com$/.test(origin)) {
    throw new Error(`get.faulk.app redirects to ${origin}, not a public Vercel Blob store`)
  }
  return origin
}

/**
 * Whether a read-write token belongs to the store at `origin`, checked before anything is
 * written (the origin of each answer is checked again afterwards). A token reads
 * `vercel_blob_rw_<storeId>_<secret>` (`parseStoreIdFromReadWriteToken` of `@vercel/blob`), and a
 * public store answers at `https://<storeId, lower case>.public.blob.vercel-storage.com`
 * (`vercel blob get-store` shows both).
 */
export function tokenMatchesOrigin(token, origin) {
  const [vercel, blob, rw, storeId = ''] = String(token).split('_')
  if (vercel !== 'vercel' || blob !== 'blob' || rw !== 'rw' || !/^[0-9A-Za-z]+$/.test(storeId)) return false
  return origin === `https://${storeId.toLowerCase()}.public.blob.vercel-storage.com`
}

/** An in-memory store, for the dry run and the tests. */
export function memoryStore(origin, { seed = new Map(), log = () => {} } = {}) {
  const blobs = new Map(seed)
  const ops = []
  return {
    blobs,
    ops,
    async head(path) {
      ops.push(['head', path])
      return blobs.has(path) ? { url: `${origin}/${path}` } : null
    },
    async read(path) {
      ops.push(['read', path])
      return blobs.has(path) ? blobs.get(path) : null
    },
    async put(path, bytes, opts) {
      ops.push(['put', path, opts])
      if (blobs.has(path) && !opts.allowOverwrite) throw new Error(`blob ${path} already exists`)
      log(`[dry-run] put ${path} ${JSON.stringify(opts)}`)
      blobs.set(path, Buffer.from(bytes))
      return { url: `${origin}/${path}` }
    },
  }
}
