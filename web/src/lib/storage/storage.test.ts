// The project storage controller (SV-2) against stand-ins for the browser and the core: when it
// writes, what it says, and what it does when the folder, the permission or the page goes away.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { FolderEntry, SaveEntry, SaveFrame, SaveMode } from '../project/core'
import type { OpenOutcome } from '../stores/open'
import {
  AUTOSAVE_DELAY_MS,
  AUTOSAVE_MAX_WAIT_MS,
  autosaveDelay,
  changedPaths,
  ProjectStorage,
  projectIdOf,
  RETRY_MS,
  type ProjectHost,
  type StorageDeps,
} from './storage.svelte'
import { StoreError, type FolderHandle, type ProjectStore, type RecentProject, type Signature } from './types'

const text = (s: string) => new TextEncoder().encode(s)

/** A folder in memory that records what is applied to it, and can be made to fail or wait. */
class FakeStore implements ProjectStore {
  files = new Map<string, string>()
  blobs = new Set<string>()
  applied: string[][] = []
  fail: StoreError | null = null
  hold: Promise<void> | null = null
  closed = false
  project = true
  copied: ProjectStore[] = []
  stamp = 1
  constructor(readonly name: string) {}

  async isEmpty() {
    return this.files.size === 0
  }
  async hasProject() {
    return this.project
  }
  async readFolder() {
    const entries: FolderEntry[] = [...this.files].map(([path, body]) => ({ path, bytes: text(body) }))
    return { entries, signature: this.signature() }
  }
  async read() {
    return new Uint8Array()
  }
  signature(): Signature {
    return Object.fromEntries([...this.files.keys()].map((path) => [path, `${this.files.get(path)!.length}:${this.stamp}`]))
  }
  async apply(entries: SaveEntry[]) {
    if (this.hold) await this.hold
    if (this.fail) throw this.fail
    this.applied.push(entries.map((e) => e.path))
    this.stamp++
    const written: Signature = {}
    for (const entry of entries) {
      if (entry.type === 'remove') this.files.delete(entry.path)
      else {
        this.files.set(entry.path, new TextDecoder().decode(entry.bytes))
        written[entry.path] = `${entry.bytes.length}:${this.stamp}`
      }
    }
    return written
  }
  async scan() {
    return this.signature()
  }
  async copyBlobsFrom(source: ProjectStore) {
    this.copied.push(source)
  }
  async cleanUp() {}
  close() {
    this.closed = true
  }
  /** Someone else edits a file. */
  edit(path: string, body: string) {
    this.files.set(path, body)
    this.stamp++
  }
}

/** The core, reduced to its revision counter: `save` gives one file, `project.json`, that carries the revision. */
class FakeHost implements ProjectHost {
  revision = 0
  clean = 0
  pending = 0
  saves: SaveMode[] = []
  opened: { name: string; entries: FolderEntry[] }[] = []
  refuseOpen = false
  frameFails = false
  id = 'project-1'
  news = 0
  dirty() {
    return this.revision !== this.clean
  }
  /** How long computing a write takes (the fake clock moves by that much). */
  cost = 0
  save(mode: SaveMode): SaveFrame {
    this.saves.push(mode)
    if (this.cost) vi.advanceTimersByTime(this.cost)
    if (this.frameFails) return { status: 'failed', message: 'does not serialize' }
    this.pending = this.revision
    if (mode === 'changed' && !this.dirty()) return { status: 'ok', entries: [] }
    return { status: 'ok', entries: [{ type: 'write', path: 'project.json', bytes: text(`rev ${this.revision}`) }] }
  }
  saved(applied: boolean) {
    if (applied) this.clean = this.pending
  }
  async openProject(name: string, entries: FolderEntry[]): Promise<OpenOutcome> {
    if (this.refuseOpen) return { status: 'refused', reason: 'corrupt' }
    this.opened.push({ name, entries })
    this.revision = this.clean = 0
    return { status: 'opened', name }
  }
  async newProject() {
    this.news++
    this.revision = this.clean = 0
    this.id = `project-${this.news + 1}`
    return true
  }
  projectId() {
    return this.id
  }
}

interface World {
  storage: ProjectStorage
  host: FakeHost
  deps: StorageDeps
  stores: Map<FolderHandle, FakeStore>
  picks: ({ status: 'chosen'; handle: FolderHandle } | { status: 'cancelled' } | { status: 'refused' })[]
  permission: Map<FolderHandle, PermissionState>
  asked: PermissionState
  locks: Set<string>
  watchers: (() => void)[]
  recents: RecentProject[]
  supported: boolean
  edit(): void
}

function handleOf(name: string): FolderHandle {
  return { name } as unknown as FolderHandle
}

function world(): World {
  const stores = new Map<FolderHandle, FakeStore>()
  const w = {
    stores,
    picks: [],
    permission: new Map(),
    asked: 'granted' as PermissionState,
    locks: new Set<string>(),
    watchers: [] as (() => void)[],
    recents: [] as RecentProject[],
    supported: true,
  } as unknown as World
  w.host = new FakeHost()
  w.deps = {
    picker: {
      get supported() {
        return w.supported
      },
      pick: async () => w.picks.shift() ?? { status: 'cancelled' },
    },
    recent: {
      list: async () => [...w.recents],
      remember: async (handle) => {
        w.recents = [{ id: handle.name, name: handle.name, handle, at: 1 }, ...w.recents.filter((r) => r.handle !== handle)]
      },
      forget: async (id) => {
        w.recents = w.recents.filter((r) => r.id !== id)
      },
    },
    openStore: async (handle) => {
      let store = stores.get(handle)
      if (!store) stores.set(handle, (store = new FakeStore(handle.name)))
      store.closed = false
      return store
    },
    permission: async (handle) => w.permission.get(handle) ?? 'granted',
    askPermission: async (handle) => {
      if (w.asked === 'granted') w.permission.set(handle, 'granted')
      return w.asked
    },
    lock: async (id) => {
      if (w.locks.has(id)) return null
      w.locks.add(id)
      return () => w.locks.delete(id)
    },
    watch: (_handle, onChange) => {
      w.watchers.push(onChange)
      return () => {
        w.watchers = w.watchers.filter((f) => f !== onChange)
      }
    },
    now: () => Date.now(),
  }
  w.storage = new ProjectStorage(w.host, w.deps)
  w.edit = () => {
    w.host.revision++
    w.storage.changed()
  }
  return w
}

/** Lets promise chains run without moving the clock. */
const settle = () => vi.advanceTimersByTimeAsync(0)

/** A world whose project was saved as into a folder called `name`. */
async function saved(name = 'song') {
  const w = world()
  const handle = handleOf(name)
  w.picks.push({ status: 'chosen', handle })
  w.edit()
  expect(await w.storage.saveAs()).toBe(true)
  return { w, handle, store: w.stores.get(handle)! }
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-10-24T12:00:00'))
})
afterEach(() => vi.useRealTimers())

describe('a project with no folder', () => {
  it('is unsaved, warns on leaving once it was changed, and writes nothing', async () => {
    const w = world()
    expect(w.storage.status).toBe('unsaved')
    expect(w.storage.unsaved).toBe(false)
    w.edit()
    expect(w.storage.unsaved).toBe(true)
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(w.host.saves).toEqual([])
    expect(w.storage.status).toBe('unsaved')
  })

  it('says so when the browser has no folder to offer, for Save as and Open, and Save', async () => {
    const w = world()
    w.supported = false
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.unsupported')
    w.storage.dismissNotice()
    expect(await w.storage.openFolder()).toBe(false)
    expect(w.storage.notice).toBe('storage.unsupported')
    w.storage.dismissNotice()
    expect(await w.storage.save()).toBe(false)
    expect(w.storage.notice).toBe('storage.unsupported')
    expect(w.storage.unsaved).toBe(false)
  })
})

describe('save as', () => {
  it('writes the whole project into the folder the user picked, then the project lives there', async () => {
    const { w, store } = await saved()
    expect(w.host.saves).toEqual(['everything'])
    expect(store.files.get('project.json')).toBe('rev 1')
    expect(w.storage.folder).toEqual({ name: 'song' })
    expect(w.storage.status).toBe('saved')
    expect(w.storage.unsaved).toBe(false)
    expect(w.storage.savedAt).toBe(Date.now())
    expect(w.locks).toEqual(new Set(['project-1']))
    expect(w.recents.map((r) => r.name)).toEqual(['song'])
  })

  it('leaves everything as it was when the user cancels, or the browser refuses the folder', async () => {
    const w = world()
    w.edit()
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBeNull()
    w.picks.push({ status: 'refused' })
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.refused')
    expect(w.storage.folder).toBeNull()
    expect(w.host.saves).toEqual([])
    expect(w.storage.unsaved).toBe(true)
  })

  it('refuses a folder that is not empty, and one that cannot be written, without moving the project', async () => {
    const w = world()
    w.edit()
    const full = handleOf('full')
    w.picks.push({ status: 'chosen', handle: full })
    w.stores.set(full, Object.assign(new FakeStore('full'), { files: new Map([['readme.txt', 'hi']]) }))
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.notEmpty')
    expect(w.stores.get(full)!.closed).toBe(true)
    expect(w.host.saves).toEqual([])

    const broken = handleOf('broken')
    w.picks.push({ status: 'chosen', handle: broken })
    w.stores.set(broken, Object.assign(new FakeStore('broken'), { fail: new StoreError('io', 'nope') }))
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.saveFailed')
    expect(w.storage.folder).toBeNull()
    expect(w.host.clean).toBe(0)
    expect(w.host.dirty()).toBe(true)
    expect(w.locks.size).toBe(0)
  })

  it('does not take a project another tab holds', async () => {
    const w = world()
    w.edit()
    w.locks.add('project-1')
    w.picks.push({ status: 'chosen', handle: handleOf('song') })
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.locked')
    expect(w.storage.folder).toBeNull()
  })

  it('from a project that has a folder writes a copy: a new identity, the blobs, and the old folder left alone', async () => {
    const { w, store: first } = await saved('first')
    const filesBefore = new Map(first.files)
    const appliedBefore = first.applied.length
    const copy = handleOf('copy')
    w.picks.push({ status: 'chosen', handle: copy })
    w.host.id = 'project-2'
    expect(await w.storage.saveAs()).toBe(true)
    expect(w.host.saves.at(-1)).toBe('copy')
    const second = w.stores.get(copy)!
    expect(second.copied).toEqual([first])
    expect(second.files.has('project.json')).toBe(true)
    expect(first.files).toEqual(filesBefore)
    expect(first.applied.length).toBe(appliedBefore)
    expect(first.closed).toBe(true)
    expect(w.storage.folder).toEqual({ name: 'copy' })
    // The edits that follow go to the copy only.
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(second.applied.length).toBe(2)
    expect(first.applied.length).toBe(appliedBefore)
  })
})

describe('autosave', () => {
  it('waits a moment after the last change, and writes once for a burst', async () => {
    const { w, store } = await saved()
    for (let i = 0; i < 5; i++) {
      w.edit()
      await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS - 100)
    }
    expect(w.storage.status).toBe('pending')
    expect(w.storage.unsaved).toBe(true)
    expect(store.applied.length).toBe(1)
    await vi.advanceTimersByTimeAsync(200)
    expect(store.applied.length).toBe(2)
    expect(store.files.get('project.json')).toBe('rev 6')
    expect(w.storage.status).toBe('saved')
    expect(w.storage.unsaved).toBe(false)
  })

  it('waits longer when a write costs the main thread a lot: about a thirtieth of it at most', async () => {
    const { w, store } = await saved()
    w.host.cost = 800
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(store.applied.length).toBe(2)
    expect(w.storage.lastWrite!.coreMs).toBeGreaterThanOrEqual(800)
    // The next one is spaced by what the last one cost.
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS * 5)
    expect(store.applied.length).toBe(2)
    expect(w.storage.status).toBe('pending')
    await vi.advanceTimersByTimeAsync(autosaveDelay(800))
    expect(store.applied.length).toBe(3)
    expect(autosaveDelay(0)).toBe(AUTOSAVE_DELAY_MS)
    expect(autosaveDelay(10_000)).toBe(60_000)
  })

  it('writes at least every ten seconds under continuous editing', async () => {
    const { w, store } = await saved()
    const before = store.applied.length
    for (let t = 0; t < AUTOSAVE_MAX_WAIT_MS; t += 1000) {
      w.edit()
      await vi.advanceTimersByTimeAsync(1000)
    }
    expect(store.applied.length).toBeGreaterThan(before)
  })

  it('keeps an edit made during a write dirty, and writes it next', async () => {
    const { w, store } = await saved()
    let release!: () => void
    store.hold = new Promise((resolve) => (release = resolve))
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.status).toBe('saving')
    expect(w.storage.unsaved).toBe(true)
    w.edit()
    store.hold = null
    release()
    await settle()
    expect(w.host.dirty()).toBe(true)
    expect(w.storage.status).toBe('pending')
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(store.files.get('project.json')).toBe('rev 3')
    expect(w.storage.status).toBe('saved')
  })

  it('writes at once when the page goes to the background', async () => {
    const { w, store } = await saved()
    w.edit()
    w.storage.sleep()
    await settle()
    expect(store.files.get('project.json')).toBe('rev 2')
    expect(w.storage.status).toBe('saved')
  })

  it('Save writes now, whatever the timer says, and a project with nothing new writes nothing', async () => {
    const { w, store } = await saved()
    w.edit()
    expect(await w.storage.save()).toBe(true)
    expect(store.files.get('project.json')).toBe('rev 2')
    expect(w.host.saves.at(-1)).toBe('forced')
    const applied = store.applied.length
    w.host.save = () => ({ status: 'ok', entries: [] })
    expect(await w.storage.save()).toBe(true)
    expect(store.applied.length).toBe(applied)
  })

  it('a project that does not serialize is an error banner, not a crash, and the changes stay', async () => {
    const { w } = await saved()
    w.edit()
    w.host.frameFails = true
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('error')
    expect(w.storage.status).toBe('failed')
    expect(w.storage.unsaved).toBe(true)
  })
})

describe('a write that fails', () => {
  it('shows the failure, keeps the changes, and tries again later by itself', async () => {
    const { w, store } = await saved()
    store.fail = new StoreError('full', 'disk full')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('error')
    expect(w.storage.status).toBe('failed')
    expect(w.storage.errorDetail).toContain('disk full')
    expect(w.storage.unsaved).toBe(true)
    expect(w.host.clean).toBe(1)
    store.fail = null
    await vi.advanceTimersByTimeAsync(RETRY_MS)
    expect(w.storage.block).toBeNull()
    expect(w.storage.status).toBe('saved')
    expect(store.files.get('project.json')).toBe('rev 2')
  })

  it('the retry button saves again', async () => {
    const { w, store } = await saved()
    store.fail = new StoreError('io', 'locked')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    store.fail = null
    expect(await w.storage.save()).toBe(true)
    expect(w.storage.status).toBe('saved')
  })
})

describe('the browser takes the permission away', () => {
  it('pauses saving with a banner, keeps the changes, and resumes after the user allows', async () => {
    const { w, handle, store } = await saved()
    w.permission.set(handle, 'prompt')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('permission')
    expect(w.storage.status).toBe('paused')
    expect(w.storage.unsaved).toBe(true)
    expect(store.applied.length).toBe(1)
    // More edits do not try again while it waits.
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(store.applied.length).toBe(1)

    // The user says no: still paused.
    w.asked = 'denied'
    expect(await w.storage.resume()).toBe(false)
    expect(w.storage.block).toBe('permission')
    // The user says yes.
    w.asked = 'granted'
    await w.storage.resume()
    expect(w.storage.block).toBeNull()
    expect(store.files.get('project.json')).toBe('rev 3')
    expect(w.storage.status).toBe('saved')
  })

  it('also learns it from a write the browser refuses, and from the permission coming back on its own', async () => {
    const { w, handle, store } = await saved()
    store.fail = new StoreError('permission', 'NotAllowedError')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('permission')
    expect(w.host.dirty()).toBe(true)
    store.fail = null
    w.permission.set(handle, 'granted')
    await w.storage.wake()
    await settle()
    expect(w.storage.block).toBeNull()
    expect(w.storage.status).toBe('saved')
  })
})

describe('opening a folder', () => {
  async function opening() {
    const w = world()
    const handle = handleOf('mine')
    const store = new FakeStore('mine')
    store.files = new Map([['project.json', JSON.stringify({ id: 'abc' })]])
    w.stores.set(handle, store)
    w.picks.push({ status: 'chosen', handle })
    return { w, handle, store }
  }

  it('replaces the project, takes the lock of its identity, and saves into that folder from then on', async () => {
    const { w, store } = await opening()
    expect(await w.storage.openFolder()).toBe(true)
    expect(w.host.opened[0]!.name).toBe('mine')
    expect(w.locks).toEqual(new Set(['abc']))
    expect(w.storage.folder).toEqual({ name: 'mine' })
    expect(w.storage.status).toBe('saved')
    expect(w.storage.savedAt).toBeNull()
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(store.files.get('project.json')).toBe('rev 1')
  })

  it('says a folder is not a project, and does not touch the current one', async () => {
    const { w, store } = await opening()
    store.project = false
    expect(await w.storage.openFolder()).toBe(false)
    expect(w.storage.notice).toBe('project.open.notAProject')
    expect(w.host.opened).toEqual([])
    expect(w.storage.folder).toBeNull()
  })

  it('gives the lock back when the project refuses the folder, and does not open one another tab holds', async () => {
    const { w } = await opening()
    w.host.refuseOpen = true
    expect(await w.storage.openFolder()).toBe(false)
    expect(w.locks.size).toBe(0)
    expect(w.storage.folder).toBeNull()
    w.host.refuseOpen = false
    w.locks.add('abc')
    w.picks.push({ status: 'chosen', handle: [...w.stores.keys()][0]! })
    expect(await w.storage.openFolder()).toBe(false)
    expect(w.storage.notice).toBe('storage.locked')
  })

  it('asks before throwing away changes that no folder holds; Cancel changes nothing, Discard goes on', async () => {
    const { w } = await opening()
    w.edit()
    const asking = w.storage.openFolder()
    await settle()
    expect(w.storage.confirm).toEqual({ action: 'open', name: null })
    w.storage.respond(false)
    expect(await asking).toBe(false)
    expect(w.host.opened).toEqual([])
    expect(w.storage.confirm).toBeNull()

    w.picks.push({ status: 'chosen', handle: [...w.stores.keys()][0]! })
    const again = w.storage.openFolder()
    await settle()
    w.storage.respond(true)
    expect(await again).toBe(true)
    expect(w.host.opened.length).toBe(1)
  })

  it('writes the changes of a project that has a folder first, and asks only when it could not', async () => {
    const { w, store: current } = await saved('current')
    const other = handleOf('other')
    const target = new FakeStore('other')
    target.files = new Map([['project.json', JSON.stringify({ id: 'xyz' })]])
    w.stores.set(other, target)
    w.edit()
    w.picks.push({ status: 'chosen', handle: other })
    expect(await w.storage.openFolder()).toBe(true)
    expect(current.files.get('project.json')).toBe('rev 2')
    expect(w.storage.confirm).toBeNull()
    expect(current.closed).toBe(true)
    expect(w.locks).toEqual(new Set(['xyz']))

    // Now the current folder cannot be written: the user is asked.
    const stuck = handleOf('stuck')
    const stuckStore = new FakeStore('stuck')
    stuckStore.files = new Map([['project.json', JSON.stringify({ id: 'stuck' })]])
    w.stores.set(stuck, stuckStore)
    target.fail = new StoreError('io', 'locked')
    w.edit()
    w.picks.push({ status: 'chosen', handle: stuck })
    const asking = w.storage.openFolder()
    await settle()
    expect(w.storage.confirm).toEqual({ action: 'open', name: 'other' })
    w.storage.respond(false)
    expect(await asking).toBe(false)
    expect(w.storage.folder).toEqual({ name: 'other' })
  })
})

describe('a new project', () => {
  it('asks when changes would be lost, and leaves no folder behind: the old one is never written again', async () => {
    const { w, store } = await saved()
    // With a folder, the changes are written, then the project is replaced.
    w.edit()
    expect(await w.storage.newProject()).toBe(true)
    expect(w.host.news).toBe(1)
    expect(store.files.get('project.json')).toBe('rev 2')
    expect(w.storage.folder).toBeNull()
    expect(w.storage.status).toBe('unsaved')
    expect(w.storage.unsaved).toBe(false)
    expect(w.locks.size).toBe(0)
    expect(store.closed).toBe(true)
    // An edit of the new project goes nowhere.
    const applied = store.applied.length
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(store.applied.length).toBe(applied)
    expect(w.storage.unsaved).toBe(true)

    // Without a folder, changed: it asks.
    const asking = w.storage.newProject()
    await settle()
    expect(w.storage.confirm).toEqual({ action: 'new', name: null })
    w.storage.respond(false)
    expect(await asking).toBe(false)
    expect(w.host.news).toBe(1)
    const again = w.storage.newProject()
    await settle()
    w.storage.respond(true)
    expect(await again).toBe(true)
    expect(w.host.news).toBe(2)
    expect(w.storage.unsaved).toBe(false)
  })

  it('a project.new run as a command forgets the folder at once', async () => {
    const { w, store } = await saved()
    w.storage.forgetFolder()
    await settle()
    expect(w.storage.folder).toBeNull()
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS)
    expect(store.applied.length).toBe(1)
  })
})

describe('the folder is edited from outside', () => {
  it('ignores its own writes, and reloads a change when nothing is unsaved', async () => {
    const { w, store } = await saved()
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    w.watchers.forEach((f) => f())
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.host.opened).toEqual([])
    expect(w.storage.block).toBeNull()

    store.edit('project.json', 'from git')
    w.watchers.forEach((f) => f())
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.host.opened.length).toBe(1)
    expect(w.storage.notice).toBe('storage.reloaded')
    expect(w.storage.status).toBe('saved')
  })

  it('with unsaved changes asks which version to keep: Keep mine writes over it, Load takes the folder', async () => {
    const { w, store } = await saved()
    w.edit()
    store.edit('project.json', 'from git')
    await w.storage.wake()
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.storage.block).toBe('external')
    expect(w.storage.status).toBe('conflict')
    expect(w.host.opened).toEqual([])
    // Nothing is written meanwhile.
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(store.files.get('project.json')).toBe('from git')

    expect(await w.storage.keepMine()).toBe(true)
    expect(w.host.saves.at(-1)).toBe('rewrite')
    expect(store.files.get('project.json')).toBe('rev 2')
    expect(w.storage.block).toBeNull()

    w.edit()
    store.edit('project.json', 'again')
    await w.storage.wake()
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.storage.block).toBe('external')
    expect(await w.storage.loadDisk()).toBe(true)
    expect(w.host.opened.length).toBe(1)
    expect(w.storage.block).toBeNull()
    expect(w.storage.unsaved).toBe(false)
  })

  it('a change seen while a write is under way is looked at once the write is done', async () => {
    const { w, store } = await saved()
    let release!: () => void
    store.hold = new Promise((resolve) => (release = resolve))
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    store.edit('notes.tsv', 'outside')
    w.watchers.forEach((f) => f())
    store.hold = null
    release()
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.host.opened.length).toBe(1)
  })
})

describe('the start of a page', () => {
  async function remembered(permission: PermissionState) {
    const w = world()
    const handle = handleOf('last')
    const store = new FakeStore('last')
    store.files = new Map([['project.json', JSON.stringify({ id: 'abc' })]])
    w.stores.set(handle, store)
    w.recents = [{ id: 'last', name: 'last', handle, at: 1 }]
    w.permission.set(handle, permission)
    return { w, handle }
  }

  it('opens the project of last time when the browser still allows it', async () => {
    const { w } = await remembered('granted')
    await w.storage.restore()
    expect(w.host.opened.map((o) => o.name)).toEqual(['last'])
    expect(w.storage.folder).toEqual({ name: 'last' })
    expect(w.storage.reopen).toBeNull()
  })

  it('offers it with a banner when the browser wants a click, and opens it on the click', async () => {
    const { w } = await remembered('prompt')
    await w.storage.restore()
    expect(w.host.opened).toEqual([])
    expect(w.storage.reopen).toEqual({ id: 'last', name: 'last' })
    w.asked = 'denied'
    expect(await w.storage.reopenLast()).toBe(false)
    expect(w.storage.notice).toBe('storage.permissionDenied')
    w.asked = 'granted'
    expect(await w.storage.reopenLast()).toBe(true)
    expect(w.storage.folder).toEqual({ name: 'last' })
    expect(w.storage.reopen).toBeNull()
  })

  it('forgets a project whose folder is gone, and does not take over a page that was already changed', async () => {
    const { w, handle } = await remembered('granted')
    w.stores.get(handle)!.project = false
    await w.storage.restore()
    expect(w.storage.notice).toBe('project.open.notAProject')
    expect(w.recents).toEqual([])

    const other = await remembered('granted')
    other.w.edit()
    await other.w.storage.restore()
    expect(other.w.host.opened).toEqual([])
  })

  it('does nothing without a recent project', async () => {
    const w = world()
    await w.storage.restore()
    expect(w.storage.reopen).toBeNull()
    expect(w.storage.folder).toBeNull()
  })
})

describe('helpers', () => {
  it('reads the identity from project.json and compares stamps', () => {
    expect(projectIdOf([{ path: 'project.json', bytes: text('{"id":"abc"}') }])).toBe('abc')
    expect(projectIdOf([{ path: 'project.json', bytes: text('{') }])).toBeNull()
    expect(projectIdOf([{ path: 'project.json', size: 3 }])).toBeNull()
    expect(projectIdOf([])).toBeNull()
    expect(changedPaths({ a: '1', b: '2' }, { a: '1', b: '3', c: '1' }).sort()).toEqual(['b', 'c'])
    expect(changedPaths({ a: '1' }, {})).toEqual(['a'])
  })
})
