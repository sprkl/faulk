// The browser's own storage as a home for projects (SV-3): where the directories are, what the page
// lists of them, and what happens when the OPFS refuses. Against a directory in memory standing in
// for `navigator.storage.getDirectory()`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryDir } from './memory-dir'
import { OpfsProjects, opfsSupported } from './opfs'
import { StoreError } from './types'

let root: MemoryDir
let storage: { getDirectory: () => Promise<unknown>; persisted?: () => Promise<boolean>; persist?: () => Promise<boolean> }

beforeEach(() => {
  root = new MemoryDir('')
  storage = { getDirectory: async () => root }
  vi.stubGlobal('navigator', { storage })
  vi.stubGlobal('FileSystemFileHandle', { prototype: { createWritable() {} } })
})
afterEach(() => vi.unstubAllGlobals())

const project = (dir: string, id: string, name: string) => root.put(`faulk/browser/${dir}/project.json`, JSON.stringify({ id, name }))

describe('opfsSupported', () => {
  it('needs the OPFS and atomic writes (createWritable)', () => {
    expect(opfsSupported()).toBe(true)
    vi.stubGlobal('FileSystemFileHandle', { prototype: {} })
    expect(opfsSupported()).toBe(false)
    vi.stubGlobal('FileSystemFileHandle', { prototype: { createWritable() {} } })
    vi.stubGlobal('navigator', { storage: {} })
    expect(opfsSupported()).toBe(false)
    vi.stubGlobal('navigator', {})
    expect(opfsSupported()).toBe(false)
  })
})

describe('the list of projects', () => {
  it('gives the identity and name from project.json, newest change first', async () => {
    project('old', 'id-old', 'Old song')
    project('new', 'id-new', 'New song')
    const { projects, incomplete } = await new OpfsProjects().list()
    expect(projects.map((p) => [p.dir, p.id, p.name])).toEqual([
      ['new', 'id-new', 'New song'],
      ['old', 'id-old', 'Old song'],
    ])
    expect(incomplete).toEqual([])
  })

  it('dates a project by its newest file, not by project.json, which a save rewrites only when it changes', async () => {
    project('a', 'id-a', 'A')
    project('b', 'id-b', 'B')
    // A note edited in A, a long time after: A is the one worked on last, though its project.json is older.
    root.put('faulk/browser/a/tracks/t1/clips/c1.notes.tsv', '60\t0\t480')
    const { projects } = await new OpfsProjects().list()
    expect(projects.map((p) => p.dir)).toEqual(['a', 'b'])
    expect(projects[0]!.modified).toBeGreaterThan(projects[1]!.modified)
  })

  it('does not look into blobs/ for the date, and names a project that has no name by its directory', async () => {
    root.put('faulk/browser/a/project.json', JSON.stringify({ id: 'id-a' }))
    root.put('faulk/browser/a/blobs/aa', 'media')
    const { projects } = await new OpfsProjects().list()
    expect(projects[0]!.name).toBe('a')
  })

  it('tells a directory with no project.json (being made, or left half made) from one whose project.json does not read', async () => {
    root.put('faulk/browser/half/blobs/aa', 'media')
    root.put('faulk/browser/broken/project.json', '{ not json')
    root.put('faulk/browser/no-id/project.json', '{}')
    root.put('faulk/browser/stray-file', 'x')
    project('ok', 'id', 'Ok')
    const { projects, incomplete } = await new OpfsProjects().list()
    expect(projects.map((p) => p.dir)).toEqual(['ok'])
    expect(incomplete).toEqual(['half'])
  })

  it('is empty where the OPFS cannot be listed', async () => {
    storage.getDirectory = async () => {
      throw new DOMException('no', 'SecurityError')
    }
    expect(await new OpfsProjects().list()).toEqual({ projects: [], incomplete: [] })
  })
})

describe('making, opening and deleting', () => {
  it('makes a directory under faulk/browser, opens it again, and deletes it with everything in it', async () => {
    const opfs = new OpfsProjects()
    const handle = await opfs.create('p1')
    expect(handle.name).toBe('p1')
    root.put('faulk/browser/p1/tracks/t/track.json', '{}')
    expect(await opfs.open('p1')).toBe(handle)
    expect(await opfs.open('nope')).toBeNull()
    await opfs.remove('p1')
    expect(root.paths()).toEqual([])
    // Deleting what is not there is not an error: two tabs may both clear it.
    await expect(opfs.remove('p1')).resolves.toBeUndefined()
  })

  it('says a full quota is a full quota and stays usable; any other refusal switches it off', async () => {
    const opfs = new OpfsProjects()
    const real = root.getDirectoryHandle.bind(root)
    root.getDirectoryHandle = async (name, options) => {
      if (name === 'faulk') throw new DOMException('full', 'QuotaExceededError')
      return real(name, options)
    }
    await expect(opfs.create('p')).rejects.toMatchObject({ code: 'full' })
    expect(opfs.available).toBe(true)

    root.getDirectoryHandle = async () => {
      throw new DOMException('private window', 'SecurityError')
    }
    await expect(opfs.create('p')).rejects.toBeInstanceOf(StoreError)
    expect(opfs.available).toBe(false)
    expect(await opfs.list()).toEqual({ projects: [], incomplete: [] })
    expect(await opfs.open('p')).toBeNull()
  })

  it('is not available at all where the OPFS is not supported', async () => {
    vi.stubGlobal('navigator', {})
    expect(new OpfsProjects().available).toBe(false)
  })
})

describe('asking the browser to keep the storage', () => {
  it('does not ask again when it is kept already, and answers what the browser says', async () => {
    const persist = vi.fn(async () => true)
    storage.persisted = async () => true
    storage.persist = persist
    expect(await new OpfsProjects().persist()).toBe(true)
    expect(persist).not.toHaveBeenCalled()

    storage.persisted = async () => false
    storage.persist = async () => false
    expect(await new OpfsProjects().persist()).toBe(false)
    storage.persist = async () => true
    expect(await new OpfsProjects().persist()).toBe(true)
  })

  it('is false, not an error, where the browser has no such call or refuses', async () => {
    expect(await new OpfsProjects().persist()).toBe(false)
    storage.persist = async () => {
      throw new Error('no')
    }
    expect(await new OpfsProjects().persist()).toBe(false)
  })
})
