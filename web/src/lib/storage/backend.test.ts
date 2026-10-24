import { describe, expect, it } from 'vitest'
import { classify, FolderBackend } from './backend'
import { MemoryDir } from './memory-dir'
import { StoreError } from './types'

const text = (s: string) => new TextEncoder().encode(s)
const write = (path: string, s: string) => ({ type: 'write' as const, path, bytes: text(s) })

function folder() {
  const dir = new MemoryDir('song')
  return { dir, store: new FolderBackend(dir.handle()) }
}

describe('FolderBackend.apply', () => {
  it('writes the files in order, makes the directories, and gives the stamps of what is not in .faulk', async () => {
    const { dir, store } = folder()
    const stamps = await store.apply([
      write('tracks/a/track.json', '{"a":1}'),
      write('project.json', '{"p":1}'),
      write('.faulk/session.json', '{}'),
    ])
    expect(dir.paths().sort()).toEqual(['.faulk/session.json', 'project.json', 'tracks/a/track.json'])
    expect(dir.text('tracks/a/track.json')).toBe('{"a":1}')
    expect(Object.keys(stamps).sort()).toEqual(['project.json', 'tracks/a/track.json'])
    expect(stamps['project.json']).toBe(`7:${(await (await dir.getFileHandle('project.json')).getFile()).lastModified}`)
  })

  it('replaces a file whole or not at all: a write that fails at close leaves the old file intact', async () => {
    const { dir, store } = folder()
    await store.apply([write('project.json', 'old')])
    dir.hooks.failClose = (file) => file === 'project.json'
    await expect(store.apply([write('project.json', 'new and longer')])).rejects.toMatchObject({ code: 'io' })
    expect(dir.text('project.json')).toBe('old')
    dir.hooks.failClose = undefined
    dir.hooks.failWrite = () => true
    await expect(store.apply([write('project.json', 'x')])).rejects.toMatchObject({ code: 'full' })
    expect(dir.text('project.json')).toBe('old')
  })

  it('makes a new file whole or not at all: it is written in .faulk/tmp and moved into place', async () => {
    const { dir, store } = folder()
    // A write that never completes (the tab goes away): the name never appears, not even empty.
    dir.hooks.failClose = () => true
    await expect(store.apply([write('tracks/a/track.json', 'x')])).rejects.toBeInstanceOf(StoreError)
    expect(dir.paths()).toEqual([])
    dir.hooks.failClose = undefined
    await store.apply([write('tracks/a/track.json', '{"a":1}')])
    expect(dir.paths()).toEqual(['tracks/a/track.json'])
    expect(dir.text('.faulk/tmp')).toBeNull()
    expect(dir.text('tracks/a/track.json')).toBe('{"a":1}')
  })

  it('where move is not available, makes the file in place, and a write that fails removes it again', async () => {
    const { dir, store } = folder()
    dir.hooks.noMove = true
    const stamps = await store.apply([write('a.json', '1')])
    expect(dir.text('a.json')).toBe('1')
    expect(Object.keys(stamps)).toEqual(['a.json'])
    dir.hooks.failClose = (file) => file === 'b.json'
    await expect(store.apply([write('b.json', '2')])).rejects.toBeInstanceOf(StoreError)
    expect(dir.paths()).toEqual(['a.json'])
  })

  it('a move the browser refuses (not allowed, invalid) falls back to making the file in place, once and for good', async () => {
    for (const refusal of ['InvalidModificationError', 'NotAllowedError']) {
      const { dir, store } = folder()
      dir.hooks.moveRejects = refusal
      await store.apply([write('a.json', '1'), write('b.json', '2')])
      expect(dir.paths().sort()).toEqual(['a.json', 'b.json'])
      expect(dir.text('a.json')).toBe('1')
    }
    // A content that cannot be written is still the caller's to hear, and leaves no file behind.
    const { dir, store } = folder()
    dir.hooks.failWrite = () => true
    await expect(store.apply([write('a.json', '1')])).rejects.toMatchObject({ code: 'full' })
    expect(dir.paths()).toEqual([])
  })

  it('stops at the first failure: what came before stays, what came after is not written', async () => {
    const { dir, store } = folder()
    // The second file cannot be closed (new files are written under a temporary name first).
    let closes = 0
    dir.hooks.failClose = () => ++closes === 2
    await expect(store.apply([write('a.json', '1'), write('b.json', '2'), write('c.json', '3')])).rejects.toBeInstanceOf(StoreError)
    // Not even an empty b.json: a new file that could not be written is not left behind.
    expect(dir.paths()).toEqual(['a.json'])
  })

  it('removes a file and the directories it leaves empty, never one that still holds files, never the root', async () => {
    const { dir, store } = folder()
    await store.apply([write('tracks/a/clips/c.json', '1'), write('tracks/a/track.json', '2'), write('project.json', '3')])
    await store.apply([{ type: 'remove', path: 'tracks/a/clips/c.json' }])
    expect(dir.paths().sort()).toEqual(['project.json', 'tracks/a/track.json'])
    await store.apply([{ type: 'remove', path: 'tracks/a/track.json' }])
    expect(dir.entries.has('tracks')).toBe(false)
    // A file that is not there is not an error.
    await store.apply([{ type: 'remove', path: 'nothing/here.json' }])
    await store.apply([{ type: 'remove', path: 'project.json' }])
    expect(dir.paths()).toEqual([])
  })
})

describe('FolderBackend.readFolder and scan', () => {
  const lane = new Uint8Array([...text('FKAUTO01'), 1, 2, 3])
  const audio = new Uint8Array(100).fill(7)

  function song() {
    const { dir, store } = folder()
    dir.put('project.json', '{"id":"x"}')
      .put('tracks/a/track.json', '{}')
      .put('blobs/lane', lane)
      .put('blobs/audio', audio)
      .put('.faulk/session.json', '{"solo":[]}')
      .put('.faulk/cache/project.snap', 'cache')
      .put('.git/config', 'git')
      .put('tracks/a/notes.tsv.crswap', 'half')
    return { dir, store }
  }

  it('reads the text files, the lane blobs and the session; lists the other blobs by size; skips the rest', async () => {
    const { store } = song()
    const { entries } = await store.readFolder()
    const byPath = Object.fromEntries(entries.map((e) => [e.path, e]))
    expect(Object.keys(byPath).sort()).toEqual(['.faulk/session.json', 'blobs/audio', 'blobs/lane', 'project.json', 'tracks/a/track.json'])
    expect('bytes' in byPath['project.json']!).toBe(true)
    expect('bytes' in byPath['.faulk/session.json']!).toBe(true)
    expect('bytes' in byPath['blobs/lane']!).toBe(true)
    expect(byPath['blobs/audio']).toEqual({ path: 'blobs/audio', size: 100 })
  })

  it('stamps what a scan sees the same way, and never .faulk, .git or a swap file', async () => {
    const { store } = song()
    const { signature } = await store.readFolder()
    expect(Object.keys(signature).sort()).toEqual(['blobs/audio', 'blobs/lane', 'project.json', 'tracks/a/track.json'])
    expect(await store.scan()).toEqual(signature)
    await store.apply([write('project.json', '{"id":"y"}')])
    expect((await store.scan())['project.json']).not.toBe(signature['project.json'])
  })

  it('knows an empty folder (a .DS_Store does not count) and a project', async () => {
    const { dir, store } = folder()
    expect(await store.isEmpty()).toBe(true)
    dir.put('.DS_Store', 'x')
    expect(await store.isEmpty()).toBe(true)
    expect(await store.hasProject()).toBe(false)
    dir.put('project.json', '{}')
    expect(await store.isEmpty()).toBe(false)
    expect(await store.hasProject()).toBe(true)
  })
})

describe('FolderBackend blobs and swap files', () => {
  it('copies the blobs the other folder lacks, once', async () => {
    const from = folder()
    const to = folder()
    from.dir.put('blobs/a', new Uint8Array([1, 2, 3])).put('blobs/b', new Uint8Array([4]))
    to.dir.put('blobs/a', new Uint8Array([9]))
    await to.store.copyBlobsFrom(from.store)
    expect([...(await to.store.read('blobs/a'))]).toEqual([9])
    expect([...(await to.store.read('blobs/b'))]).toEqual([4])
    // A folder with no blobs is nothing to copy.
    await folder().store.copyBlobsFrom(folder().store)
  })

  it('reads a range of a file', async () => {
    const { dir, store } = folder()
    dir.put('blobs/x', new Uint8Array([0, 1, 2, 3, 4, 5]))
    expect([...(await store.read('blobs/x', 2, 4))]).toEqual([2, 3])
    await expect(store.read('blobs/missing')).rejects.toMatchObject({ code: 'missing' })
  })

  it('removes the swap files an interrupted write left, and nothing else', async () => {
    const { dir, store } = folder()
    dir.put('tracks/a/notes.tsv.crswap', 'half').put('tracks/a/notes.tsv', 'whole').put('project.json.crswap', 'x')
    dir.put('.faulk/tmp/1-a', 'half a new file').put('.faulk/session.json', '{}')
    await store.cleanUp()
    expect(dir.paths().sort()).toEqual(['.faulk/session.json', 'tracks/a/notes.tsv'])
  })
})

describe('classify', () => {
  it('maps what the file system says to what the page does', () => {
    const named = (name: string) => classify(new DOMException('x', name)).code
    expect(named('NotAllowedError')).toBe('permission')
    expect(named('SecurityError')).toBe('permission')
    expect(named('QuotaExceededError')).toBe('full')
    expect(named('NotFoundError')).toBe('missing')
    expect(named('AbortError')).toBe('io')
    expect(classify(new Error('boom')).code).toBe('io')
    expect(classify('text').code).toBe('io')
    const store = new StoreError('full', 'x')
    expect(classify(store)).toBe(store)
  })
})
