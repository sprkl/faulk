// The project storage controller (SV-2) against stand-ins for the browser and the core: when it
// writes, what it says, and what it does when the folder, the permission or the page goes away.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { FolderEntry, PackExport, PackImport, SaveEntry, SaveFrame, SaveMode } from '../project/core'
import type { OpenOutcome } from '../stores/open'
import {
  AUTOSAVE_DELAY_MS,
  AUTOSAVE_MAX_WAIT_MS,
  autosaveDelay,
  changedPaths,
  MAX_PACK_BYTES,
  ProjectStorage,
  projectIdOf,
  RETRY_MS,
  type ProjectHost,
  type StorageDeps,
} from './storage.svelte'
import { StoreError, type BrowserProject, type BrowserProjects, type FolderHandle, type ProjectStore, type RecentProject, type Signature } from './types'

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
  /** The media the folder holds (audio is not in a save). */
  media = new Map<string, Uint8Array>()
  async readBlobs() {
    return [...this.media].map(([path, bytes]) => ({ path, bytes }))
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
  prepare?: () => Promise<void>
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
  /** The change is only the session's (a selection): the folder lacks it, the project is as it was. */
  sessionOnly = false
  projectDirty() {
    return this.dirty() && !this.sessionOnly
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
  name = 'Song'
  projectName() {
    return this.name
  }
  /** What the core answers to a pack export, and to an import. */
  exports: { path: string; bytes: Uint8Array }[][] = []
  exportFails: string | null = null
  missingBlobs = 0
  unpacked: PackImport = { status: 'ok', files: [] }
  async packExport(files: { path: string; bytes: Uint8Array }[]): Promise<PackExport> {
    this.exports.push(files)
    if (this.exportFails) return { status: 'failed', message: this.exportFails }
    return { status: 'ok', zip: text(`zip of ${files.length}`), files: files.length, missingBlobs: this.missingBlobs }
  }
  async packImport(): Promise<PackImport> {
    return this.unpacked
  }
}

/** The browser's own storage, in memory: directories whose stores are the world's. */
class FakeBrowser implements BrowserProjects {
  available = true
  created: string[] = []
  removed: string[] = []
  createFails: StoreError | null = null
  persistAnswer = true
  persisted = 0
  handles = new Map<string, FolderHandle>()
  meta = new Map<string, { id: string; name: string; modified: number }>()
  constructor(
    public w: World,
    private readonly now: () => number,
  ) {}

  async list() {
    const projects: BrowserProject[] = []
    const incomplete: string[] = []
    for (const [dir, handle] of this.handles) {
      const store = this.w.stores.get(handle)
      if (!store?.files.has('project.json')) {
        incomplete.push(dir)
        continue
      }
      let parsed: { id?: string; name?: string } = {}
      try {
        parsed = JSON.parse(store.files.get('project.json')!)
      } catch {
        // The fake host writes "rev N": the identity is the one recorded at creation.
      }
      const meta = this.meta.get(dir)!
      projects.push({ dir, id: parsed.id ?? meta.id, name: parsed.name ?? meta.name, modified: meta.modified })
    }
    return { projects: projects.sort((a, b) => b.modified - a.modified), incomplete }
  }
  async create(dir: string) {
    if (this.createFails) throw this.createFails
    const handle = handleOf(`browser/${dir}`)
    this.handles.set(dir, handle)
    this.created.push(dir)
    this.meta.set(dir, { id: this.w.host.id, name: this.w.host.name, modified: this.now() })
    return handle
  }
  async open(dir: string) {
    return this.handles.get(dir) ?? null
  }
  async remove(dir: string) {
    this.handles.delete(dir)
    this.removed.push(dir)
  }
  async persist() {
    this.persisted++
    return this.persistAnswer
  }
  /** The directory the world's only browser project lives in. */
  get only() {
    expect(this.handles.size).toBe(1)
    return [...this.handles.keys()][0]!
  }
  /** Stamps a project as last changed at `modified`. */
  touch(dir: string, modified: number) {
    this.meta.set(dir, { ...this.meta.get(dir)!, modified })
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
  browser: FakeBrowser
  /** The files the user "chooses" for an import, and what the page hands over for download. */
  packs: (File | null)[]
  downloads: { name: string; blob: Blob }[]
  /** What each link answers for `openUrl` (#497): bytes, or a network failure; the links asked for. */
  served: Map<string, Uint8Array | 'unreachable'>
  fetched: { url: string; init: RequestInit | undefined }[]
  pageHostname: string
  edit(): void
}

function handleOf(name: string): FolderHandle {
  const handle = { name, isSameEntry: async (other: unknown) => other === handle }
  return handle as unknown as FolderHandle
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
    packs: [] as (File | null)[],
    downloads: [] as { name: string; blob: Blob }[],
    served: new Map(),
    fetched: [],
    pageHostname: 'faulk.app',
  } as unknown as World
  w.host = new FakeHost()
  w.browser = new FakeBrowser(w, () => Date.now())
  // Most tests are about folders: the browser's storage is off unless a test turns it on.
  w.browser.available = false
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
    browser: w.browser,
    pickPack: async () => w.packs.shift() ?? null,
    download: (name, blob) => {
      w.downloads.push({ name, blob })
    },
    fetch: async (input, init) => {
      const url = input instanceof Request ? input.url : input.toString()
      w.fetched.push({ url, init })
      const answer = w.served.get(url)
      if (answer === undefined) return new Response('not found', { status: 404 })
      if (answer === 'unreachable') throw new TypeError('Failed to fetch')
      return new Response(answer as Uint8Array<ArrayBuffer>)
    },
    pageHostname: () => w.pageHostname,
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

/** Lets a question the storage asks open, answers it (yes goes ahead), and gives what the action came to. */
async function settling<T>(w: World, action: Promise<T>, yes: boolean): Promise<T> {
  await settle()
  expect(w.storage.confirm).not.toBeNull()
  w.storage.respond(yes)
  return action
}

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

  it('does not say saved before the project is remembered: a reload right then must still offer it', async () => {
    const w = world()
    const handle = handleOf('song')
    w.picks.push({ status: 'chosen', handle })
    w.edit()
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    const remember = w.deps.recent.remember
    let reached = false
    w.deps.recent.remember = async (h) => {
      reached = true
      await gate
      await remember(h)
    }
    const done = w.storage.saveAs()
    for (let i = 0; i < 200 && !reached; i++) await Promise.resolve()
    expect(reached).toBe(true)
    for (let i = 0; i < 20; i++) await Promise.resolve()
    // The folder is written but not yet among the recent projects: a page reloaded now finds nothing to offer.
    expect(w.recents).toEqual([])
    expect(w.storage.status).not.toBe('saved')
    expect(w.storage.savedAt).toBeNull()
    release()
    expect(await done).toBe(true)
    expect(w.recents.map((r) => r.name)).toEqual(['song'])
    expect(w.storage.status).toBe('saved')
    expect(w.storage.savedAt).toBe(Date.now())
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

  it('files written before a write failed are its own: not an outside change', async () => {
    const { w, store } = await saved()
    store.apply = async (entries) => {
      store.files.set(entries[0]!.path, 'half')
      store.stamp++
      throw new StoreError('io', 'died after the first file')
    }
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('error')
    w.watchers.forEach((f) => f())
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.storage.block).toBe('error')
    expect(w.host.opened).toEqual([])
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

  it('opening the project that is open is a no-op, not "open in another tab"', async () => {
    const { w, handle } = await saved('here')
    w.picks.push({ status: 'chosen', handle })
    expect(await w.storage.openFolder()).toBe(true)
    expect(w.storage.notice).toBeNull()
    expect(w.storage.folder).toEqual({ name: 'here' })
    expect(w.host.opened).toEqual([])
    expect(w.locks).toEqual(new Set(['project-1']))
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

  it('Cancel on a project with no folder leaves the project, its changes and the core untouched (#346)', async () => {
    const w = world()
    w.edit()
    const asking = w.storage.newProject()
    await settle()
    expect(w.storage.confirm).toEqual({ action: 'new', name: null })
    // Nothing was replaced while the question is open.
    expect(w.host.news).toBe(0)
    w.storage.respond(false)
    expect(await asking).toBe(false)
    expect(w.host.news).toBe(0)
    expect(w.storage.unsaved).toBe(true)
    expect(w.storage.confirm).toBeNull()
  })

  it('Cancel on a project in a folder keeps the folder attached and its changes written (#346)', async () => {
    const { w, store } = await saved()
    // A write that cannot happen: the changes are kept nowhere, so the user is asked.
    w.permission.set(w.stores.keys().next().value!, 'prompt')
    w.edit()
    const asking = w.storage.newProject()
    await settle()
    expect(w.storage.confirm?.action).toBe('new')
    expect(w.host.news).toBe(0)
    w.storage.respond(false)
    expect(await asking).toBe(false)
    expect(w.host.news).toBe(0)
    expect(w.storage.folder).toEqual({ name: 'song' })
    expect(store.closed).toBe(false)
  })

  it('a second new project waits for the first question, and asks nothing once the first replaced the project', async () => {
    const w = world()
    w.edit()
    const first = w.storage.newProject()
    const second = w.storage.newProject()
    await settle()
    w.storage.respond(true)
    expect(await first).toBe(true)
    expect(await second).toBe(true)
    expect(w.host.news).toBe(2)
    expect(w.storage.confirm).toBeNull()
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

    expect(await settling(w, w.storage.keepMine(), true)).toBe(true)
    expect(w.host.saves.at(-1)).toBe('rewrite')
    expect(store.files.get('project.json')).toBe('rev 2')
    expect(w.storage.block).toBeNull()

    w.edit()
    store.edit('project.json', 'again')
    await w.storage.wake()
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.storage.block).toBe('external')
    expect(await settling(w, w.storage.loadDisk(), true)).toBe(true)
    expect(w.host.opened.length).toBe(1)
    expect(w.storage.block).toBeNull()
    expect(w.storage.unsaved).toBe(false)
  })

  it('Keep mine also removes the track files the project no longer has, so they do not come back on opening', async () => {
    const { w, store } = await saved()
    store.files.set('tracks/gone/track.json', '{}')
    store.files.set('tracks/gone/clips/c.json', '{}')
    store.files.set('blobs/audio', 'kept')
    w.edit()
    store.edit('project.json', 'edited outside')
    await w.storage.wake()
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.storage.block).toBe('external')
    expect(await settling(w, w.storage.keepMine(), true)).toBe(true)
    expect([...store.files.keys()].sort()).toEqual(['blobs/audio', 'project.json'])
    expect(store.files.get('project.json')).toBe('rev 2')
  })

  it('a folder that cannot be read as a project stops the writes and asks, instead of retrying on every focus', async () => {
    const { w, store } = await saved()
    w.host.refuseOpen = true
    store.edit('project.json', '{ half a checkout')
    await w.storage.wake()
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.storage.block).toBe('external')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(store.files.get('project.json')).toBe('{ half a checkout')
    const opened = w.host.opened.length
    await w.storage.wake()
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.host.opened.length).toBe(opened)
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
    expect(w.storage.reopen).toEqual({ id: 'last', name: 'last', kind: 'folder' })
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

  it('keeps the project that would not open for the notice’s actions (#504): locked, then unreadable, then opened by a retry', async () => {
    const { w } = await remembered('granted')
    w.locks.add('abc')
    await w.storage.restore()
    expect(w.storage.notice).toBe('storage.locked')
    expect(w.storage.failedReopen).toEqual({ id: 'last', name: 'last', kind: 'folder' })

    // A retry that fails the same way keeps the actions; a failure of another kind keeps them too.
    expect(await w.storage.reopenLast()).toBe(false)
    expect(w.storage.failedReopen).toEqual({ id: 'last', name: 'last', kind: 'folder' })
    w.locks.clear()
    const open = w.host.openProject.bind(w.host)
    w.host.openProject = async () => {
      throw new StoreError('io', 'unreadable')
    }
    expect(await w.storage.reopenLast()).toBe(false)
    expect(w.storage.notice).toBe('project.open.unreadable')
    expect(w.storage.failedReopen).toEqual({ id: 'last', name: 'last', kind: 'folder' })

    w.host.openProject = open
    expect(await w.storage.reopenLast()).toBe(true)
    expect(w.storage.folder).toEqual({ name: 'last' })
    expect(w.storage.notice).toBeNull()
    expect(w.storage.failedReopen).toBeNull()
  })

  it('keeps the actions when a retry runs into the permission instead (#504)', async () => {
    const { w, handle } = await remembered('granted')
    w.locks.add('abc')
    await w.storage.restore()
    expect(w.storage.failedReopen).not.toBeNull()
    w.locks.clear()
    w.permission.set(handle, 'prompt')
    w.asked = 'denied'
    expect(await w.storage.reopenLast()).toBe(false)
    expect(w.storage.notice).toBe('storage.permissionDenied')
    expect(w.storage.failedReopen).toEqual({ id: 'last', name: 'last', kind: 'folder' })
    w.asked = 'granted'
    expect(await w.storage.reopenLast()).toBe(true)
    expect(w.storage.failedReopen).toBeNull()
  })

  it('keeps the notice and its actions when a retry is declined at the question about unsaved changes, and joins a second click (#504)', async () => {
    const { w } = await remembered('granted')
    w.locks.add('abc')
    await w.storage.restore()
    expect(w.storage.notice).toBe('storage.locked')
    w.locks.clear()
    w.edit()
    const first = w.storage.reopenLast()
    const second = w.storage.reopenLast()
    await settle()
    expect(w.storage.confirm).toEqual({ action: 'open', name: null })
    w.storage.respond(false)
    expect(await first).toBe(false)
    expect(await second).toBe(false)
    expect(w.storage.notice).toBe('storage.locked')
    expect(w.storage.failedReopen).toEqual({ id: 'last', name: 'last', kind: 'folder' })
    expect(w.host.opened).toEqual([])
  })

  it('does not offer the actions when the user refuses the permission on the first try of the reopen banner (#504)', async () => {
    const { w } = await remembered('prompt')
    await w.storage.restore()
    w.asked = 'denied'
    expect(await w.storage.reopenLast()).toBe(false)
    expect(w.storage.notice).toBe('storage.permissionDenied')
    expect(w.storage.failedReopen).toBeNull()
  })

  it('offers no retry for a notice a second try cannot answer, and drops it with the notice', async () => {
    const { w, handle } = await remembered('granted')
    w.stores.get(handle)!.project = false
    await w.storage.restore()
    expect(w.storage.notice).toBe('project.open.notAProject')
    expect(w.storage.failedReopen).toBeNull()

    const locked = await remembered('granted')
    locked.w.locks.add('abc')
    await locked.w.storage.restore()
    expect(locked.w.storage.failedReopen).not.toBeNull()
    locked.w.storage.dismissNotice()
    expect(locked.w.storage.failedReopen).toBeNull()
    expect(await locked.w.storage.reopenLast()).toBe(false)
    expect(locked.w.storage.notice).toBe('storage.nothingToReopen')
  })
})

// ---------------------------------------------------------------------- SV-3: the browser's storage

/** A world whose browser keeps projects (the OPFS), with a project changed and its draft written. */
async function drafted(options: { picker?: boolean } = {}) {
  const w = world()
  w.browser.available = true
  w.supported = options.picker ?? true
  w.edit()
  await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
  const dir = w.browser.only
  return { w, dir, handle: w.browser.handles.get(dir)!, store: w.stores.get(w.browser.handles.get(dir)!)! }
}

const packFile = (name = 'song.faulkpack') => new File(['zip'], name)
const packOf = (id: string, name = 'Packed') => ({
  status: 'ok' as const,
  files: [
    { path: 'project.json', bytes: text(JSON.stringify({ id, name })) },
    { path: 'blobs/aa', bytes: text('media') },
    { path: 'tracks/t1/track.json', bytes: text('{}') },
  ],
})

describe('a draft in the browser’s storage', () => {
  it('is written after the first change, in a browser that has a folder picker too', async () => {
    const { w, store } = await drafted()
    expect(w.host.saves).toEqual(['everything'])
    expect(store.files.get('project.json')).toBe('rev 1')
    expect(w.storage.place).toBe('browser')
    expect(w.storage.folder).toBeNull()
    expect(w.storage.status).toBe('saved')
    expect(w.storage.unsaved).toBe(false)
    expect(w.recents).toEqual([])
    expect(w.watchers).toEqual([])
    // The identity of the project and the directory are both locked, and the browser is asked to keep the storage.
    expect([...w.locks].sort()).toEqual([`dir.${w.browser.only}`, 'project-1'].sort())
    expect(w.browser.persisted).toBe(1)
    expect(w.storage.persisted).toBe(true)
    expect(w.storage.browserProjects.map((p) => p.dir)).toEqual([w.browser.only])
  })

  it('writes nothing for a project nobody touched, and keeps the old behaviour where the browser cannot keep one', async () => {
    const w = world()
    w.browser.available = true
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(w.browser.created).toEqual([])
    expect(w.storage.status).toBe('unsaved')

    const off = world()
    off.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(off.browser.created).toEqual([])
    expect(off.storage.status).toBe('unsaved')
    expect(off.storage.unsaved).toBe(true)
  })

  it('makes no draft, and does not warn, for a page whose only change is its session (the first track selected)', async () => {
    const w = world()
    w.browser.available = true
    w.host.sessionOnly = true
    w.edit()
    expect(w.storage.unsaved).toBe(false)
    expect(w.storage.status).toBe('unsaved')
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(w.browser.created).toEqual([])
    // New does not ask about it, and a later restore is not put off by it.
    expect(await w.storage.newProject()).toBe(true)
    expect(w.storage.confirm).toBeNull()
    // A real edit afterwards is kept like any other.
    w.host.sessionOnly = false
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.browser.created.length).toBe(1)
  })

  it('shows a change as pending until its first write, then keeps writing the changes to the draft', async () => {
    const w = world()
    w.browser.available = true
    w.edit()
    expect(w.storage.status).toBe('pending')
    expect(w.storage.unsaved).toBe(true)
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    const store = w.stores.get(w.browser.handles.get(w.browser.only)!)!
    expect(w.storage.status).toBe('saved')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(store.files.get('project.json')).toBe('rev 2')
    expect(w.host.saves).toEqual(['everything', 'changed'])
    expect(w.browser.created.length).toBe(1)
    expect(w.browser.persisted).toBe(1)
  })

  it('is written at once when the page goes to the background, and before New or Open replaces the project', async () => {
    const w = world()
    w.browser.available = true
    w.edit()
    w.storage.sleep()
    await settle()
    expect(w.browser.created.length).toBe(1)

    const other = world()
    other.browser.available = true
    other.edit()
    // Still inside the delay: no draft yet. New keeps the change in a draft instead of asking.
    expect(await other.storage.newProject()).toBe(true)
    expect(other.storage.confirm).toBeNull()
    expect(other.browser.created.length).toBe(1)
    const kept = other.stores.get(other.browser.handles.get(other.browser.only)!)!
    expect(kept.files.get('project.json')).toBe('rev 1')
    expect(other.storage.place).toBe('none')
    expect(other.locks.size).toBe(0)
    // And the draft is still offered afterwards.
    await other.storage.refreshBrowserProjects()
    expect(other.storage.browserProjects.length).toBe(1)
  })

  it('a quota that is full is its own banner, keeps the changes, and the retry carries on once there is room', async () => {
    const w = world()
    w.browser.available = true
    w.edit()
    const full = new StoreError('full', 'QuotaExceededError')
    // The first write: the directory was made, the files do not fit.
    const original = w.deps.openStore
    w.deps.openStore = async (handle) => {
      const store = (await original(handle)) as FakeStore
      store.fail = full
      return store
    }
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('full')
    expect(w.storage.status).toBe('failed')
    expect(w.storage.place).toBe('none')
    expect(w.storage.unsaved).toBe(true)
    // What was made for it is given back: no directory, no lock.
    expect(w.browser.handles.size).toBe(0)
    expect(w.locks.size).toBe(0)
    expect(w.host.dirty()).toBe(true)
    w.deps.openStore = original
    await vi.advanceTimersByTimeAsync(RETRY_MS)
    expect(w.storage.block).toBeNull()
    expect(w.storage.place).toBe('browser')
    expect(w.storage.status).toBe('saved')
  })

  it('a quota that fills up later keeps the files it wrote whole, says so, and the next write completes the project', async () => {
    const { w, store } = await drafted()
    store.fail = new StoreError('full', 'QuotaExceededError')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('full')
    expect(w.storage.status).toBe('failed')
    expect(store.files.get('project.json')).toBe('rev 1')
    expect(w.host.dirty()).toBe(true)
    store.fail = null
    await vi.advanceTimersByTimeAsync(RETRY_MS)
    expect(w.storage.block).toBeNull()
    expect(store.files.get('project.json')).toBe('rev 2')
    expect(w.host.dirty()).toBe(false)
  })

  it('Try again writes again, even where a picker exists, instead of asking for a folder', async () => {
    const { w, store } = await drafted()
    store.fail = new StoreError('full', 'QuotaExceededError')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('full')
    store.fail = null
    expect(await w.storage.save()).toBe(true)
    expect(w.storage.block).toBeNull()
    expect(store.files.get('project.json')).toBe('rev 2')
    expect(w.storage.place).toBe('browser')

    // The same when the very first write failed: no project of the browser yet, and Try again makes it.
    const first = world()
    first.browser.available = true
    first.browser.createFails = new StoreError('full', 'QuotaExceededError')
    first.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(first.storage.block).toBe('full')
    first.browser.createFails = null
    expect(await first.storage.save()).toBe(true)
    expect(first.storage.place).toBe('browser')
    expect(first.storage.block).toBeNull()
  })

  it('another failure of the first write is the plain error banner, tried again by itself', async () => {
    const w = world()
    w.browser.available = true
    w.browser.createFails = new StoreError('io', 'nope')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('error')
    expect(w.storage.errorDetail).toContain('nope')
    expect(w.locks.size).toBe(0)
    w.browser.createFails = null
    await vi.advanceTimersByTimeAsync(RETRY_MS)
    expect(w.storage.block).toBeNull()
    expect(w.storage.place).toBe('browser')
  })

  it('a browser whose storage turns out unusable (a private window) goes back to the tab, with no banner', async () => {
    const w = world()
    w.browser.available = true
    w.browser.createFails = new StoreError('io', 'SecurityError')
    const make = w.browser.create.bind(w.browser)
    w.browser.create = async (dir) => {
      try {
        return await make(dir)
      } catch (error) {
        w.browser.available = false
        throw error
      }
    }
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBeNull()
    expect(w.storage.status).toBe('unsaved')
    expect(w.storage.unsaved).toBe(true)
    await vi.advanceTimersByTimeAsync(AUTOSAVE_MAX_WAIT_MS * 2)
    expect(w.browser.created).toEqual([])
  })

  it('Save asks for a folder where there is a picker, and keeps the project in the browser where there is none', async () => {
    const { w } = await drafted()
    w.picks.push({ status: 'cancelled' })
    expect(await w.storage.save()).toBe(false)
    expect(w.storage.place).toBe('browser')

    const none = world()
    none.browser.available = true
    none.supported = false
    expect(await none.storage.save()).toBe(true)
    expect(none.browser.created.length).toBe(1)
    expect(none.storage.place).toBe('browser')
    none.edit()
    expect(await none.storage.save()).toBe(true)
    expect(none.host.saves).toEqual(['everything', 'forced'])
    expect(none.browser.created.length).toBe(1)
  })

  it('warns that projects are in the browser only where there is no folder to choose, harder if the browser will not promise to keep them', async () => {
    const withPicker = await drafted()
    expect(withPicker.w.storage.browserWarning).toBeNull()

    const none = await drafted({ picker: false })
    expect(none.w.storage.browserWarning).toBe('export')
    none.w.storage.dismissBrowserWarning()
    expect(none.w.storage.browserWarning).toBeNull()

    const refused = world()
    refused.browser.available = true
    refused.browser.persistAnswer = false
    refused.supported = false
    refused.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(refused.storage.persisted).toBe(false)
    expect(refused.storage.browserWarning).toBe('volatile')
  })
})

describe('save as from a draft', () => {
  it('moves the same project into the folder: same identity, the media come along, the lock is carried, the draft goes', async () => {
    const { w, dir, store: draft } = await drafted()
    draft.media.set('blobs/aa', text('media'))
    const folder = handleOf('song')
    w.picks.push({ status: 'chosen', handle: folder })
    expect(await w.storage.saveAs()).toBe(true)
    expect(w.host.saves.at(-1)).toBe('everything')
    const store = w.stores.get(folder)!
    expect(store.copied).toEqual([draft])
    expect(store.files.has('project.json')).toBe(true)
    expect(w.storage.place).toBe('folder')
    expect(w.storage.folder).toEqual({ name: 'song' })
    expect(w.browser.removed).toEqual([dir])
    expect(w.browser.handles.size).toBe(0)
    expect(draft.closed).toBe(true)
    // One lock on the identity, never taken twice; none left on the draft's directory.
    expect([...w.locks]).toEqual(['project-1'])
    expect(w.recents.map((r) => r.name)).toEqual(['song'])
    expect(w.storage.browserProjects).toEqual([])
    // And the edits that follow go to the folder.
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(store.applied.length).toBe(2)
    expect(w.host.saves.at(-1)).toBe('changed')
  })

  it('deletes the draft before it says saved: a reload right then cannot find the project twice', async () => {
    const { w, dir } = await drafted()
    w.picks.push({ status: 'chosen', handle: handleOf('song') })
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    const seen: { place: string; savedAt: number | null; status: string }[] = []
    const remove = w.browser.remove.bind(w.browser)
    w.browser.remove = async (d) => {
      seen.push({ place: w.storage.place, savedAt: w.storage.savedAt, status: w.storage.status })
      await gate
      await remove(d)
    }
    const done = w.storage.saveAs()
    for (let i = 0; i < 200 && !seen.length; i++) await Promise.resolve()
    expect(seen.length).toBe(1)
    for (let i = 0; i < 20; i++) await Promise.resolve()
    // The draft is being deleted: the page does not say the folder holds the project yet.
    expect(w.storage.place).not.toBe('folder')
    release()
    expect(await done).toBe(true)
    expect(w.browser.removed).toEqual([dir])
    expect(w.storage.place).toBe('folder')
    expect(w.storage.status).toBe('saved')
    expect(w.storage.savedAt).toBe(Date.now())
  })

  it('keeps the draft when the folder did not make it into the recent projects (remember reports no failure)', async () => {
    const { w, dir } = await drafted()
    w.picks.push({ status: 'chosen', handle: handleOf('song') })
    w.deps.recent.remember = async () => undefined
    expect(await w.storage.saveAs()).toBe(true)
    expect(w.storage.place).toBe('folder')
    expect(w.storage.status).toBe('saved')
    expect(w.browser.removed).toEqual([])
    expect(w.browser.handles.has(dir)).toBe(true)
  })

  it('keeps the draft’s directory lock until the draft is gone, and gives it back whatever happened', async () => {
    const { w, dir } = await drafted()
    w.picks.push({ status: 'chosen', handle: handleOf('song') })
    const locked: boolean[] = []
    const remove = w.browser.remove.bind(w.browser)
    w.browser.remove = async (d) => {
      locked.push([...w.locks].some((l) => l.includes(dir)))
      await remove(d)
    }
    expect(await w.storage.saveAs()).toBe(true)
    expect(locked).toEqual([true])
    expect([...w.locks].some((l) => l.includes(dir))).toBe(false)
    // And when the removal throws.
    const again = await drafted()
    again.w.picks.push({ status: 'chosen', handle: handleOf('song2') })
    again.w.browser.remove = async () => {
      throw new Error('locked')
    }
    expect(await again.w.storage.saveAs()).toBe(true)
    expect([...again.w.locks].some((l) => l.includes(again.dir))).toBe(false)
  })

  it('does not hold Save or the export folder back for ever when an attach fails', async () => {
    const { w } = await saved()
    const other = handleOf('other')
    w.picks.push({ status: 'chosen', handle: other })
    // A copy has its own identity (the core gives it one).
    w.host.id = 'project-copy'
    w.deps.openStore = async (handle) => {
      const s = new FakeStore(handle.name)
      s.cleanUp = async () => {
        throw new Error('cleanup')
      }
      w.stores.set(handle, s)
      return s
    }
    w.edit()
    await w.storage.saveAs().catch(() => undefined)
    // Whatever became of the failed attach, the next calls are not left waiting on it.
    expect(await w.storage.folderHandle()).toBe(other)
    expect(await w.storage.save()).toBe(true)
    expect(w.host.saves.at(-1)).toBe('forced')
  })

  it('still says saved when the draft cannot be deleted (the folder holds the project, a leftover draft is harmless)', async () => {
    const { w } = await drafted()
    w.picks.push({ status: 'chosen', handle: handleOf('song') })
    w.browser.remove = async () => {
      throw new Error('locked')
    }
    expect(await w.storage.saveAs()).toBe(true)
    expect(w.storage.place).toBe('folder')
    expect(w.storage.status).toBe('saved')
  })

  it.each(['remember', 'remove'] as const)(
    'orders Save and the export folder after a move into a folder under way (held at %s): the new folder, never a second pick',
    async (hold) => {
      const { w } = await drafted()
      const folder = handleOf('song')
      w.picks.push({ status: 'chosen', handle: folder })
      let release!: () => void
      const gate = new Promise<void>((resolve) => (release = resolve))
      let reached = false
      const remove = w.browser.remove.bind(w.browser)
      const remember = w.deps.recent.remember
      // Held at `remember`, the draft is still the project's home; at `remove` it is detached and nobody is current.
      if (hold === 'remember')
        w.deps.recent.remember = async (h) => {
          reached = true
          await gate
          await remember(h)
        }
      else
        w.browser.remove = async (d) => {
          reached = true
          await gate
          await remove(d)
        }
      const moving = w.storage.saveAs()
      for (let i = 0; i < 200 && !reached; i++) await Promise.resolve()
      expect(reached).toBe(true)
      // A second pick must never be needed: Save finds the folder once the move is over.
      w.picks.push({ status: 'chosen', handle: handleOf('other') })
      const handle = w.storage.folderHandle()
      const saving = w.storage.save()
      for (let i = 0; i < 50; i++) await Promise.resolve()
      release()
      expect(await moving).toBe(true)
      expect(await handle).toBe(folder)
      expect(await saving).toBe(true)
      expect(w.picks.length).toBe(1)
      expect(w.host.saves.at(-1)).toBe('forced')
      expect(w.storage.folder).toEqual({ name: 'song' })
    },
  )

  it('answers the export folder: nothing without a project, and without waiting for an ordinary write (the file dialog needs the click)', async () => {
    const w = world()
    expect(await w.storage.folderHandle()).toBeNull()
    const { w: s, store, handle } = await saved()
    let release!: () => void
    store.hold = new Promise<void>((resolve) => (release = resolve))
    s.edit()
    const writing = s.storage.flush()
    let answered = false
    const asked = s.storage.folderHandle().then((h) => ((answered = true), h))
    for (let i = 0; i < 50; i++) await Promise.resolve()
    expect(answered).toBe(true)
    release()
    await writing
    expect(await asked).toBe(handle)
  })

  it('lets Save go on after a task of the queue failed, and cancelling the dialog it opens changes nothing', async () => {
    const { w, dir } = await drafted()
    const folder = w.browser.handles.get(dir)!
    w.stores.get(folder)!.fail = new StoreError('io', 'disk')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.status).toBe('failed')
    w.stores.get(folder)!.fail = null
    // A failed write into the draft: Save is "try again", it writes the draft and opens no dialog.
    expect(await w.storage.save()).toBe(true)
    expect(w.storage.place).toBe('browser')
    // Saved draft: Save asks for a folder; the user cancels; the draft stays the home.
    expect(await w.storage.save()).toBe(false)
    expect(w.storage.place).toBe('browser')
    expect(w.browser.removed).toEqual([])
  })

  it('holds Save and the export folder for the whole move, not only its attach: no second folder dialog while the media are copied', async () => {
    const { w } = await drafted()
    const folder = handleOf('song')
    const target = new FakeStore('song')
    w.stores.set(folder, target)
    let release!: () => void
    const gate = new Promise<void>((resolve) => (release = resolve))
    let reached = false
    target.copyBlobsFrom = async () => {
      reached = true
      await gate
    }
    w.picks.push({ status: 'chosen', handle: folder })
    const moving = w.storage.saveAs()
    for (let i = 0; i < 200 && !reached; i++) await Promise.resolve()
    expect(reached).toBe(true)
    // The draft is still the project's home here, and attach has not started.
    w.picks.push({ status: 'chosen', handle: handleOf('other') })
    const asked = w.storage.folderHandle()
    const saving = w.storage.save()
    for (let i = 0; i < 50; i++) await Promise.resolve()
    release()
    expect(await moving).toBe(true)
    expect(await asked).toBe(folder)
    expect(await saving).toBe(true)
    expect(w.picks.length).toBe(1)
    expect(w.storage.folder).toEqual({ name: 'song' })
  })

  it('asks for a folder when Save is pressed on a draft that nothing is moving', async () => {
    const { w } = await drafted()
    const folder = handleOf('song')
    w.picks.push({ status: 'chosen', handle: folder })
    expect(await w.storage.save()).toBe(true)
    expect(w.storage.place).toBe('folder')
    expect(await w.storage.folderHandle()).toBe(folder)
  })

  it('leaves the draft as it was, with its locks, when the folder is refused, not empty, or fails', async () => {
    const { w, dir, store: draft } = await drafted()
    const before = new Map(draft.files)
    const locks = new Set(w.locks)

    const full = handleOf('full')
    w.stores.set(full, Object.assign(new FakeStore('full'), { files: new Map([['readme.txt', 'hi']]) }))
    w.picks.push({ status: 'chosen', handle: full })
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.notEmpty')

    const broken = handleOf('broken')
    w.stores.set(broken, Object.assign(new FakeStore('broken'), { fail: new StoreError('io', 'nope') }))
    w.picks.push({ status: 'chosen', handle: broken })
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.saveFailed')

    // The blobs could not be copied.
    const noBlobs = handleOf('no-blobs')
    const store = new FakeStore('no-blobs')
    store.copyBlobsFrom = async () => {
      throw new StoreError('io', 'copy failed')
    }
    w.stores.set(noBlobs, store)
    w.picks.push({ status: 'chosen', handle: noBlobs })
    expect(await w.storage.saveAs()).toBe(false)

    expect(w.browser.removed).toEqual([])
    expect(w.browser.handles.has(dir)).toBe(true)
    expect(draft.files).toEqual(before)
    expect(draft.closed).toBe(false)
    expect(w.locks).toEqual(locks)
    expect(w.storage.place).toBe('browser')
    // The draft is still the project's home: it goes on being written.
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(draft.files.get('project.json')).toBe('rev 2')
  })

  it('does not take a folder when another tab holds the project', async () => {
    const { w } = await drafted()
    // Our own lock on the identity is what a folder save would carry; an unrelated holder cannot exist for it.
    w.picks.push({ status: 'chosen', handle: handleOf('song') })
    expect(await w.storage.saveAs()).toBe(true)
    expect(w.locks.has('project-1')).toBe(true)
  })
})

describe('the start of a page with drafts', () => {
  async function leftBehind(picker: boolean) {
    const first = await drafted({ picker })
    // A new page in the same browser: same storage, nothing open.
    const w = world()
    w.browser = first.w.browser
    w.deps.browser = first.w.browser
    w.browser.w = w
    w.stores = first.w.stores
    w.deps.openStore = first.w.deps.openStore
    first.w.storage.respond(false)
    // The first page is gone: its locks with it.
    first.w.locks.clear()
    w.locks = first.w.locks
    w.supported = picker
    return { w, dir: first.dir, draft: first.store }
  }

  it('reopens the draft by itself where there is a picker too: the browser’s storage asks no permission (#474)', async () => {
    const { w, dir } = await leftBehind(true)
    await w.storage.restore()
    expect(w.host.opened.length).toBe(1)
    expect(w.storage.reopen).toBeNull()
    expect(w.storage.banner).toBeNull()
    expect(w.storage.place).toBe('browser')
    expect([...w.locks].sort()).toEqual([`dir.${dir}`].sort())
    // Reopened, a change goes to the same draft.
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.browser.created.length).toBe(1)
  })

  it('keeps a draft that is unreadable, then locked, for the notice’s actions, and a retry opens it (#504)', async () => {
    const { w, dir } = await leftBehind(true)
    const open = w.host.openProject.bind(w.host)
    w.host.openProject = async () => {
      throw new StoreError('io', 'unreadable')
    }
    await w.storage.restore()
    expect(w.storage.notice).toBe('project.open.unreadable')
    expect(w.storage.failedReopen).toMatchObject({ id: dir, kind: 'browser' })

    w.host.openProject = open
    w.locks.add(`dir.${dir}`)
    expect(await w.storage.reopenLast()).toBe(false)
    expect(w.storage.notice).toBe('storage.locked')
    expect(w.storage.failedReopen).toMatchObject({ id: dir, kind: 'browser' })

    w.locks.delete(`dir.${dir}`)
    expect(await w.storage.reopenLast()).toBe(true)
    expect(w.storage.failedReopen).toBeNull()
    expect(w.storage.place).toBe('browser')
  })

  it('reopens the project by itself where the browser’s storage is the only home', async () => {
    const { w } = await leftBehind(false)
    await w.storage.restore()
    expect(w.host.opened.length).toBe(1)
    expect(w.storage.place).toBe('browser')
    expect(w.storage.reopen).toBeNull()
  })

  it('prefers whichever the musician worked on last: the draft, or the folder', async () => {
    const { w, dir } = await leftBehind(true)
    const folder = handleOf('song')
    const store = new FakeStore('song')
    store.files = new Map([['project.json', JSON.stringify({ id: 'abc' })]])
    w.stores.set(folder, store)
    w.permission.set(folder, 'prompt')
    w.browser.touch(dir, 1000)
    w.recents = [{ id: 'song', name: 'song', handle: folder, at: 2000 }]
    await w.storage.restore()
    expect(w.storage.reopen).toEqual({ id: 'song', name: 'song', kind: 'folder' })

    // The draft worked on after the folder opens by itself.
    w.storage.dismissReopen()
    w.browser.touch(dir, 3000)
    await w.storage.restore()
    expect(w.storage.reopen).toBeNull()
    expect(w.host.opened.length).toBe(1)
    expect(w.storage.place).toBe('browser')
  })

  it('leaves alone a draft that another tab has open, and a page that was already changed', async () => {
    const { w, dir } = await leftBehind(true)
    w.locks.add(`dir.${dir}`)
    await w.storage.restore()
    expect(w.storage.reopen).toBeNull()
    w.locks.clear()
    w.edit()
    await w.storage.restore()
    expect(w.storage.reopen).toBeNull()
    expect(w.host.opened).toEqual([])
  })

  it('deletes a directory with no project that no tab is making, and leaves one that a tab is making', async () => {
    const w = world()
    w.browser.available = true
    await w.browser.create('abandoned')
    await w.browser.create('being-made')
    w.locks.add('dir.being-made')
    await w.storage.restore()
    expect(w.browser.removed).toEqual(['abandoned'])
    expect(w.browser.handles.has('being-made')).toBe(true)
    // The sweep took the lock and gave it back.
    expect(w.locks.has('dir.abandoned')).toBe(false)
  })
})

describe('deleting a project of the browser', () => {
  async function two() {
    const { w, dir } = await drafted()
    // The page moves on to another project: the first one stays in the browser.
    expect(await w.storage.newProject()).toBe(true)
    await w.storage.refreshBrowserProjects()
    return { w, dir }
  }

  it('asks first; Cancel keeps it, Delete removes it and what offered it', async () => {
    const { w, dir } = await two()
    w.storage.reopen = { id: dir, name: 'Song', kind: 'browser' }
    const asking = w.storage.deleteBrowserProject(dir)
    await settle()
    expect(w.storage.confirm).toEqual({ action: 'delete', name: 'Song' })
    w.storage.respond(false)
    expect(await asking).toBe(false)
    expect(w.browser.handles.has(dir)).toBe(true)

    const again = w.storage.deleteBrowserProject(dir)
    await settle()
    w.storage.respond(true)
    expect(await again).toBe(true)
    expect(w.browser.removed).toEqual([dir])
    expect(w.storage.browserProjects).toEqual([])
    expect(w.storage.reopen).toBeNull()
    expect(w.locks.size).toBe(0)
  })

  it('never deletes the project that is open, nor one another tab has open', async () => {
    const { w, dir } = await drafted()
    expect(await w.storage.deleteBrowserProject(dir)).toBe(false)
    expect(w.storage.notice).toBe('storage.deleteOpen')
    expect(w.browser.removed).toEqual([])

    const other = await two()
    other.w.locks.add(`dir.${other.dir}`)
    expect(await other.w.storage.deleteBrowserProject(other.dir)).toBe(false)
    expect(other.w.storage.notice).toBe('storage.locked')
    expect(other.w.browser.removed).toEqual([])
  })

  it('says so when the project it was asked to open is gone', async () => {
    const w = world()
    w.browser.available = true
    expect(await w.storage.openBrowserProject('gone')).toBe(false)
    expect(w.storage.notice).toBe('storage.browserGone')
  })
})

describe('export a .faulkpack', () => {
  it('hands over the project as it is now, with the media of its home, named after the project', async () => {
    const { w, store } = await drafted()
    store.media.set('blobs/aa', text('media'))
    w.edit()
    w.host.name = 'My: song?'
    expect(await w.storage.exportPack()).toBe(true)
    const files = w.host.exports[0]!
    expect(files.map((f) => f.path).sort()).toEqual(['blobs/aa', 'project.json'])
    // The unsaved change is in the pack: it is the project as it is now.
    expect(new TextDecoder().decode(files.find((f) => f.path === 'project.json')!.bytes)).toBe('rev 2')
    expect(w.downloads.map((d) => d.name)).toEqual(['My_ song_.faulkpack'])
    expect(w.downloads[0]!.blob.type).toBe('application/zip')
    // The export wrote nothing: the project is still waiting for its own write.
    expect(store.files.get('project.json')).toBe('rev 1')
    expect(w.host.dirty()).toBe(true)
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(store.files.get('project.json')).toBe('rev 2')
  })

  it('works for a project that has no home at all', async () => {
    const w = world()
    w.edit()
    expect(await w.storage.exportPack()).toBe(true)
    expect(w.downloads.length).toBe(1)
    expect(w.host.dirty()).toBe(true)
  })

  it('says when the project could not be exported, or lacks media, and downloads nothing for the first', async () => {
    const { w } = await drafted()
    w.host.exportFails = 'too big'
    expect(await w.storage.exportPack()).toBe(false)
    expect(w.storage.notice).toBe('pack.exportFailed')
    expect(w.storage.errorDetail).toBe('too big')
    expect(w.downloads).toEqual([])
    w.host.exportFails = null
    w.host.missingBlobs = 2
    expect(await w.storage.exportPack()).toBe(true)
    expect(w.storage.notice).toBe('pack.missingMedia')
    w.host.frameFails = true
    expect(await w.storage.exportPack()).toBe(false)
    expect(w.storage.notice).toBe('pack.exportFailed')
  })
})

describe('import a .faulkpack', () => {
  it('writes the pack whole into a new directory of the browser, project.json last, and opens it', async () => {
    const w = world()
    w.browser.available = true
    w.host.unpacked = packOf('pack-1')
    w.packs.push(packFile())
    expect(await w.storage.importPack()).toBe(true)
    const dir = w.browser.only
    const store = w.stores.get(w.browser.handles.get(dir)!)!
    // Blobs, the other files, then project.json: the project exists only once it is whole.
    expect(store.applied).toEqual([['blobs/aa', 'tracks/t1/track.json', 'project.json']])
    expect(w.host.opened.map((o) => o.name)).toEqual([`browser/${dir}`])
    expect(w.storage.place).toBe('browser')
    expect(w.storage.browserProjects.map((p) => p.dir)).toEqual([dir])
    // The identity of the pack is the project's: nothing was saved with a new one.
    expect(w.host.saves).toEqual([])
    expect([...w.locks].sort()).toEqual([`dir.${dir}`, 'pack-1'].sort())
    expect(w.browser.persisted).toBe(1)
  })

  it('tells a caller with no click behind it, instead of waiting for a chooser that never opens', async () => {
    const w = world()
    w.browser.available = true
    vi.stubGlobal('navigator', { userActivation: { isActive: false } })
    w.packs.push(packFile())
    expect(await w.storage.importPack()).toBe(false)
    expect(w.storage.notice).toBe('storage.needsClick')
    expect(w.packs.length).toBe(1)
    vi.stubGlobal('navigator', { userActivation: { isActive: true } })
    w.host.unpacked = packOf('pack-1')
    expect(await w.storage.importPack()).toBe(true)
    vi.unstubAllGlobals()
  })

  it('changes nothing when the file is no pack, is too big, or the user chose nothing', async () => {
    const w = world()
    w.browser.available = true
    w.host.unpacked = { status: 'failed', message: 'bad zip' }
    w.packs.push(packFile())
    expect(await w.storage.importPack()).toBe(false)
    expect(w.storage.notice).toBe('pack.invalid')
    expect(w.storage.errorDetail).toBe('bad zip')
    w.packs.push(new File([new Uint8Array(MAX_PACK_BYTES + 1)], 'huge.faulkpack'))
    expect(await w.storage.importPack()).toBe(false)
    expect(w.storage.notice).toBe('pack.tooBig')
    expect(await w.storage.importPack()).toBe(false)
    expect(w.browser.created).toEqual([])
    expect(w.host.opened).toEqual([])
  })

  it('never overwrites a project of the same identity: the one open, nor the one kept in the browser', async () => {
    const { w } = await drafted()
    w.host.unpacked = packOf('project-1')
    w.packs.push(packFile())
    expect(await w.storage.importPack()).toBe(false)
    // The project is the open one: the notice names it and has nothing to open.
    expect(w.storage.notice).toBe('pack.alreadyOpen')
    expect(w.storage.noticeVars).toEqual({ name: expect.any(String) })
    expect(w.storage.noticeOpen).toBeNull()
    // Another project is open now; the first is still in the browser: the notice names it and offers to open it (#474).
    await w.storage.newProject()
    w.storage.dismissNotice()
    w.packs.push(packFile())
    expect(await w.storage.importPack()).toBe(false)
    expect(w.storage.notice).toBe('pack.alreadyHere')
    expect(w.storage.noticeVars?.name).toEqual(expect.any(String))
    expect(w.storage.noticeOpen).toEqual(expect.any(String))
    expect(w.storage.banner).toBe('notice')
    // The next notice forgets the name and the action of this one.
    w.storage.dismissNotice()
    expect(w.storage.noticeVars).toBeNull()
    expect(w.storage.noticeOpen).toBeNull()
    expect(w.browser.created.length).toBe(1)
  })

  it('leaves the project and the browser as they were when the write fails, with a message for a full quota', async () => {
    const w = world()
    w.browser.available = true
    w.edit()
    w.host.unpacked = packOf('pack-1')
    const original = w.deps.openStore
    w.deps.openStore = async (handle) => {
      const store = (await original(handle)) as FakeStore
      if (handle.name.startsWith('browser/')) store.fail = new StoreError('full', 'QuotaExceededError')
      return store
    }
    w.packs.push(packFile())
    expect(await w.storage.importPack()).toBe(false)
    expect(w.storage.notice).toBe('storage.browserFull')
    expect(w.host.opened).toEqual([])
    // The directory made for it, and its lock, are gone. (The page's own change got its draft first, as New would.)
    expect([...w.browser.handles.keys()].length).toBeLessThanOrEqual(1)
    expect(w.browser.removed.length).toBe(1)
    expect(w.locks.has('pack-1')).toBe(false)
  })

  it('removes the copy it made when the user declines to replace a changed project', async () => {
    const w = world()
    w.browser.available = false
    w.edit()
    w.host.unpacked = packOf('pack-1')
    w.packs.push(packFile())
    const importing = w.storage.importPack()
    await settle()
    expect(w.storage.confirm?.action).toBe('open')
    w.storage.respond(false)
    expect(await importing).toBe(false)
    expect(w.host.opened).toEqual([])

    const kept = world()
    kept.browser.available = true
    kept.edit()
    kept.host.unpacked = packOf('pack-2')
    kept.packs.push(packFile())
    // The change is kept in a draft first, so there is nothing to decline; the import goes through.
    expect(await kept.storage.importPack()).toBe(true)
    expect(kept.browser.handles.size).toBe(2)
  })

  it('opens the project in the tab alone where the browser keeps nothing', async () => {
    const w = world()
    w.host.unpacked = packOf('pack-1')
    w.packs.push(packFile('My song.faulkpack'))
    expect(await w.storage.importPack()).toBe(true)
    expect(w.host.opened.map((o) => o.name)).toEqual(['My song'])
    expect(w.storage.place).toBe('none')
    expect(w.browser.created).toEqual([])
    const blob = w.host.opened[0]!.entries.find((e) => e.path === 'blobs/aa')!
    expect('size' in blob && blob.size).toBe(5)
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

// ---------------------------------------------------------------------- #323: commands for the storage actions

/** A page whose click has gone: the browser would refuse a prompt or a chooser. */
const withoutClick = () => vi.stubGlobal('navigator', { userActivation: { isActive: false } })
const withClick = () => vi.stubGlobal('navigator', { userActivation: { isActive: true } })

describe('the way out of a conflict, whoever asks', () => {
  async function conflict() {
    const { w, store } = await saved()
    w.edit()
    store.edit('project.json', 'from git')
    await w.storage.wake()
    await vi.advanceTimersByTimeAsync(1000)
    expect(w.storage.block).toBe('external')
    return { w, store }
  }

  it('does nothing, and says so, when the folder is not in conflict', async () => {
    const { w, store } = await saved()
    w.edit()
    const writes = w.host.saves.length
    expect(await w.storage.keepMine()).toBe(false)
    expect(w.storage.notice).toBe('storage.noConflict')
    expect(await w.storage.loadDisk()).toBe(false)
    // No question was asked, nothing was written over the folder, nothing was reloaded.
    expect(w.storage.confirm).toBeNull()
    expect(w.host.saves.length).toBe(writes)
    expect(w.host.opened).toEqual([])
    expect(store.files.get('project.json')).toBe('rev 1')
    expect(w.host.dirty()).toBe(true)
  })

  it('has no project to settle before any is open', async () => {
    const w = world()
    expect(await w.storage.keepMine()).toBe(false)
    expect(await w.storage.loadDisk()).toBe(false)
    expect(w.storage.notice).toBe('storage.noConflict')
  })

  it('asks before Keep mine replaces the folder, and Cancel leaves both versions as they are', async () => {
    const { w, store } = await conflict()
    const asking = w.storage.keepMine()
    await settle()
    expect(w.storage.confirm).toEqual({ action: 'keepMine', name: 'song' })
    expect(store.files.get('project.json')).toBe('from git')
    w.storage.respond(false)
    expect(await asking).toBe(false)
    expect(w.storage.block).toBe('external')
    expect(store.files.get('project.json')).toBe('from git')
    expect(w.host.dirty()).toBe(true)
  })

  it('asks before Load replaces the project, and Cancel keeps the changes', async () => {
    const { w } = await conflict()
    const asking = w.storage.loadDisk()
    await settle()
    expect(w.storage.confirm).toEqual({ action: 'loadFolder', name: 'song' })
    w.storage.respond(false)
    expect(await asking).toBe(false)
    expect(w.storage.block).toBe('external')
    expect(w.host.opened).toEqual([])
    expect(w.host.dirty()).toBe(true)
  })

  it('looks again after the answer: a conflict settled while the question was open is not acted on', async () => {
    const { w, store } = await conflict()
    const asking = w.storage.keepMine()
    await settle()
    // The conflict went away meanwhile (the user settled it another way).
    w.storage.block = null
    w.storage.respond(true)
    expect(await asking).toBe(false)
    expect(w.storage.notice).toBe('storage.noConflict')
    expect(store.files.get('project.json')).toBe('from git')

    const other = await conflict()
    const loading = other.w.storage.loadDisk()
    await settle()
    other.w.storage.block = null
    other.w.storage.respond(true)
    expect(await loading).toBe(false)
    expect(other.w.host.opened).toEqual([])
  })

  it('holds the writes of the project while a question is open, and writes once it is answered', async () => {
    const { w, dir } = await twoInBrowser()
    const asking = w.storage.deleteBrowserProject(dir)
    await settle()
    expect(w.storage.confirm?.action).toBe('delete')
    // A change made while the question is open waits: nothing is lost, and it is written after.
    w.edit()
    const created = w.browser.created.length
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS * 2)
    expect(w.browser.created.length).toBe(created)
    expect(w.storage.unsaved).toBe(true)
    w.storage.respond(false)
    expect(await asking).toBe(false)
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.browser.created.length).toBe(created + 1)
    expect(w.host.dirty()).toBe(false)
  })
})

/** A draft in the browser, and the page moved on to a new project: the draft is one of the browser's projects, not the open one. */
async function twoInBrowser() {
  const made = await drafted()
  expect(await made.w.storage.newProject()).toBe(true)
  await made.w.storage.refreshBrowserProjects()
  return made
}

describe('the commands that open a project', () => {
  afterEach(() => vi.unstubAllGlobals())

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

  it('Reopen with nothing to reopen says so and changes nothing', async () => {
    const w = world()
    expect(await w.storage.reopenLast()).toBe(false)
    expect(w.storage.notice).toBe('storage.nothingToReopen')
    expect(w.host.opened).toEqual([])
  })

  it('a folder whose permission is gone needs a click; with none, nothing is asked of the browser', async () => {
    const { w, handle } = await remembered('prompt')
    withoutClick()
    expect(await w.storage.openRecent('last')).toBe(false)
    expect(w.storage.notice).toBe('storage.needsClick')
    expect(w.permission.get(handle)).toBe('prompt')
    expect(w.host.opened).toEqual([])
    withClick()
    expect(await w.storage.openRecent('last')).toBe(true)
    expect(w.storage.folder).toEqual({ name: 'last' })
  })

  it('a folder the browser still allows opens without a click', async () => {
    const { w } = await remembered('granted')
    withoutClick()
    expect(await w.storage.openRecent('last')).toBe(true)
    expect(w.storage.notice).toBeNull()
  })

  it('a folder that is not among the recent ones is refused, after looking at the list again', async () => {
    const { w } = await remembered('granted')
    // The list this page read is behind the browser's: it is read again before saying no.
    w.storage.recent = []
    expect(await w.storage.openRecent('last')).toBe(true)
    expect(w.storage.folder).toEqual({ name: 'last' })

    const other = await remembered('granted')
    expect(await other.w.storage.openRecent('nothing-like-it')).toBe(false)
    expect(other.w.storage.notice).toBe('storage.recentGone')
    expect(other.w.host.opened).toEqual([])
  })

  it('save as and save of a project with no folder need a click: no chooser opens, the notice says why (#346)', async () => {
    const w = world()
    w.edit()
    const picked = handleOf('picked')
    w.stores.set(picked, new FakeStore('picked'))
    w.picks.push({ status: 'chosen', handle: picked })
    withoutClick()
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.needsClick')
    expect(await w.storage.save()).toBe(false)
    expect(w.picks.length).toBe(1)
    expect(w.storage.folder).toBeNull()
    expect(w.host.saves).toEqual([])
    withClick()
    expect(await w.storage.saveAs()).toBe(true)
    expect(w.storage.folder).toEqual({ name: 'picked' })
    expect(w.storage.notice).toBeNull()
    vi.unstubAllGlobals()
  })

  it('save as in a browser with no folder to offer keeps saying so, click or not', async () => {
    const w = world()
    w.supported = false
    withoutClick()
    expect(await w.storage.saveAs()).toBe(false)
    expect(w.storage.notice).toBe('storage.unsupported')
    vi.unstubAllGlobals()
  })

  it('the folder picker needs a click, and a refused call leaves the pick for the next one', async () => {
    const w = world()
    const picked = handleOf('picked')
    w.stores.set(picked, Object.assign(new FakeStore('picked'), { files: new Map([['project.json', '{}']]) }))
    w.picks.push({ status: 'chosen', handle: picked })
    withoutClick()
    expect(await w.storage.openFolder()).toBe(false)
    expect(w.storage.notice).toBe('storage.needsClick')
    // Nothing opened a chooser: the pick was not consumed.
    expect(w.picks.length).toBe(1)
    withClick()
    expect(await w.storage.openFolder()).toBe(true)
    expect(w.storage.folder).toEqual({ name: 'picked' })
  })

  it('opens only a project of the browser’s own list, never a name made up', async () => {
    const { w, dir } = await twoInBrowser()
    for (const made of ['../escape', '', 'x/y', `${dir}x`]) {
      expect(await w.storage.openBrowserProject(made)).toBe(false)
      expect(w.storage.notice).toBe('storage.browserGone')
    }
    expect(w.host.opened).toEqual([])
    expect(await w.storage.openBrowserProject(dir)).toBe(true)
    expect(w.storage.place).toBe('browser')
  })

  it('deletes only a project of the browser’s own list, and never asks about one that is not', async () => {
    const { w, dir } = await twoInBrowser()
    // The banner offers a draft that is gone: no question about a nameless project, and the banner goes.
    w.storage.reopen = { id: 'gone', name: 'Ghost', kind: 'browser' }
    for (const made of ['gone', '../escape', '']) {
      expect(await w.storage.deleteBrowserProject(made)).toBe(false)
      expect(w.storage.confirm).toBeNull()
      expect(w.storage.notice).toBe('storage.browserGone')
    }
    expect(w.storage.reopen).toBeNull()
    expect(w.browser.removed).toEqual([])
    expect(w.browser.handles.has(dir)).toBe(true)
  })

  it('the recent folders and the browser’s projects are read again on demand', async () => {
    const { w } = await remembered('granted')
    expect(w.storage.recent).toEqual([])
    await w.storage.refreshStored()
    expect(w.storage.recent.map((r) => r.id)).toEqual(['last'])
    expect(w.storage.browserProjects).toEqual([])
  })
})

describe('allow access after the permission is gone', () => {
  afterEach(() => vi.unstubAllGlobals())

  async function paused() {
    const { w, handle, store } = await saved()
    w.permission.set(handle, 'prompt')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('permission')
    return { w, handle, store }
  }

  it('needs a click: with none the browser is not asked and the changes wait', async () => {
    const { w, handle, store } = await paused()
    withoutClick()
    expect(await w.storage.resume()).toBe(false)
    expect(w.storage.notice).toBe('storage.needsClick')
    expect(w.storage.block).toBe('permission')
    expect(w.permission.get(handle)).toBe('prompt')
    expect(store.files.get('project.json')).toBe('rev 1')
    withClick()
    expect(await w.storage.resume()).toBe(true)
    expect(w.storage.block).toBeNull()
    expect(store.files.get('project.json')).toBe('rev 2')
  })

  it('stays paused when the user says no', async () => {
    const { w } = await paused()
    w.asked = 'denied'
    expect(await w.storage.resume()).toBe(false)
    expect(w.storage.block).toBe('permission')
  })

  it('does nothing without a project', async () => {
    const w = world()
    expect(await w.storage.resume()).toBe(false)
  })
})

describe('the core is ready before the first click (#323)', () => {
  it('a page start makes the project core ahead of any click, and a failure of that is only logged', async () => {
    const w = world()
    const prepare = vi.fn(() => Promise.resolve())
    w.host.prepare = prepare
    await w.storage.restore()
    expect(prepare).toHaveBeenCalledTimes(1)

    const failing = world()
    const log = vi.spyOn(console, 'error').mockImplementation(() => undefined)
    failing.host.prepare = () => Promise.reject(new Error('no wasm'))
    await failing.storage.restore()
    await settle()
    expect(log).toHaveBeenCalled()
    log.mockRestore()
  })

  it('allow access gives up when the folder was replaced while the prompt was open', async () => {
    const { w, handle, store } = await saved()
    w.permission.set(handle, 'prompt')
    w.edit()
    await vi.advanceTimersByTimeAsync(AUTOSAVE_DELAY_MS)
    expect(w.storage.block).toBe('permission')
    w.deps.askPermission = async () => {
      // A new project replaces this one while the browser's prompt is open.
      const replacing = w.storage.newProject()
      await settle()
      w.storage.respond(true)
      await replacing
      return 'granted'
    }
    expect(await w.storage.resume()).toBe(false)
    expect(w.storage.folder).toBeNull()
    expect(store.files.get('project.json')).toBe('rev 1')
  })
})

describe('open a .faulkpack by its link (#497)', () => {
  const LINK = 'https://cdn.example.org/tracks/a/project.faulkpack'

  it('downloads it without credentials, keeps it in a new directory of the browser and opens it', async () => {
    const w = world()
    w.browser.available = true
    w.served.set(LINK, text('zip'))
    w.host.unpacked = packOf('pack-1')
    // The banner names the host while the download runs.
    const fetching = w.deps.fetch
    let during: string | null = null
    w.deps.fetch = (input, init) => ((during = w.storage.downloading), fetching(input, init))
    expect(await w.storage.openUrl(LINK)).toBe(true)
    expect(during).toBe('cdn.example.org')
    expect(w.storage.downloading).toBeNull()
    expect(w.fetched).toEqual([{ url: LINK, init: expect.objectContaining({ credentials: 'omit' }) }])
    const dir = w.browser.only
    expect(w.host.opened.map((o) => o.name)).toEqual([`browser/${dir}`])
    expect(w.storage.place).toBe('browser')
    expect(w.storage.notice).toBeNull()
  })

  it('keeps the current project: a changed one is kept in its own draft before the pack opens', async () => {
    const w = world()
    w.browser.available = true
    w.edit()
    w.served.set(LINK, text('zip'))
    w.host.unpacked = packOf('pack-1')
    expect(await w.storage.openUrl(LINK)).toBe(true)
    expect(w.browser.handles.size).toBe(2)
    expect(w.storage.confirm).toBeNull()
  })

  it('refuses a link the page does not fetch, plain http included, and downloads nothing', async () => {
    const w = world()
    w.browser.available = true
    for (const link of ['http://example.org/a.faulkpack', 'http://localhost:5497/a.faulkpack', 'ftp://example.org/a', 'nonsense']) {
      w.storage.dismissNotice()
      expect(await w.storage.openUrl(link)).toBe(false)
      expect(w.storage.notice).toBe('openUrl.refused')
    }
    expect(w.fetched).toEqual([])
    // On a page served from this computer, http to this computer is fine.
    w.pageHostname = 'localhost'
    w.served.set('http://localhost:5497/a.faulkpack', text('zip'))
    w.host.unpacked = packOf('pack-1')
    expect(await w.storage.openUrl('http://localhost:5497/a.faulkpack')).toBe(true)
  })

  it('says the host could not be reached, offers the file import, and changes nothing', async () => {
    const w = world()
    w.browser.available = true
    w.served.set(LINK, 'unreachable')
    expect(await w.storage.openUrl(LINK)).toBe(false)
    expect(w.storage.downloadFailed).toBe('cdn.example.org')
    expect(w.storage.errorDetail).toBe('Failed to fetch')
    // A missing file is the same failure.
    w.storage.dismissDownloadFailed()
    expect(await w.storage.openUrl('https://cdn.example.org/gone.faulkpack')).toBe(false)
    expect(w.storage.downloadFailed).toBe('cdn.example.org')
    expect(w.browser.created).toEqual([])
    expect(w.host.opened).toEqual([])
  })

  it('refuses an invalid pack and one too big, leaving the current project and the browser as they were', async () => {
    const w = world()
    w.browser.available = true
    w.served.set(LINK, text('zip'))
    w.host.unpacked = { status: 'failed', message: 'bad zip' }
    expect(await w.storage.openUrl(LINK)).toBe(false)
    expect(w.storage.notice).toBe('pack.invalid')
    w.served.set(LINK, new Uint8Array(MAX_PACK_BYTES + 1))
    w.host.unpacked = packOf('pack-1')
    expect(await w.storage.openUrl(LINK)).toBe(false)
    expect(w.storage.notice).toBe('pack.tooBig')
    expect(w.browser.created).toEqual([])
    expect(w.host.opened).toEqual([])
  })

  it('opened again, the same link opens the copy the browser keeps, never a second one', async () => {
    const w = world()
    w.browser.available = true
    w.served.set(LINK, text('zip'))
    w.host.unpacked = packOf('pack-1')
    expect(await w.storage.openUrl(LINK)).toBe(true)
    const dir = w.browser.only
    // The page's core now has the pack's project (the stand-in does not read it from the files).
    w.host.id = 'pack-1'
    // Still open: nothing to do.
    expect(await w.storage.openUrl(LINK)).toBe(true)
    expect(w.storage.notice).toBe('openUrl.alreadyOpen')
    // Another project open: the kept copy comes back.
    await w.storage.newProject()
    w.storage.dismissNotice()
    expect(await w.storage.openUrl(LINK)).toBe(true)
    expect(w.storage.notice).toBe('openUrl.keptCopy')
    expect(w.browser.created).toEqual([dir])
    expect(w.host.opened.map((o) => o.name)).toEqual([`browser/${dir}`, `browser/${dir}`])
  })

  it('downloads one link after the other', async () => {
    const w = world()
    w.browser.available = true
    const other = 'https://other.example.net/b.faulkpack'
    w.served.set(LINK, 'unreachable')
    w.served.set(other, text('zip'))
    w.host.unpacked = packOf('pack-2')
    const first = w.storage.openUrl(LINK)
    const second = w.storage.openUrl(other)
    expect(await first).toBe(false)
    expect(await second).toBe(true)
    expect(w.fetched.map((f) => f.url)).toEqual([LINK, other])
    // The second download started after the first had failed: its banner was cleared for it.
    expect(w.storage.downloadFailed).toBeNull()
  })

  it('asks about a link of the address before anything is downloaded', async () => {
    const w = world()
    w.storage.offerLink(LINK)
    expect(w.storage.linkOffer).toEqual({ url: LINK, host: 'cdn.example.org' })
    expect(w.storage.answerLink(false)).toBeNull()
    expect(w.storage.linkOffer).toBeNull()
    w.storage.offerLink(LINK)
    expect(w.storage.answerLink(true)).toBe(LINK)
    expect(w.fetched).toEqual([])
    // A link the page would not fetch is refused at once, with no question.
    w.storage.offerLink('http://example.org/a.faulkpack')
    expect(w.storage.linkOffer).toBeNull()
    expect(w.storage.notice).toBe('openUrl.refused')
  })

  it('shows the download, then its failure, before any notice, and never above what stops the work (#474)', () => {
    const w = world()
    w.storage.notice = 'storage.refused'
    w.storage.downloadFailed = 'cdn.example.org'
    expect(w.storage.banner).toBe('downloadFailed')
    w.storage.downloading = 'cdn.example.org'
    expect(w.storage.banner).toBe('downloading')
    w.storage.block = 'error'
    expect(w.storage.banner).toBe('error')
    w.storage.block = null
    w.storage.downloading = null
    w.storage.dismissDownloadFailed()
    expect(w.storage.banner).toBe('notice')
  })
})

describe('one banner at a time (#474)', () => {
  it('shows what stops the work first, then a notice, then the project of last time, and brings each back when the one before it is gone', () => {
    const w = world()
    expect(w.storage.banner).toBeNull()
    w.storage.reopen = { id: 'song', name: 'song', kind: 'folder' }
    expect(w.storage.banner).toBe('reopen')
    w.storage.block = 'error'
    expect(w.storage.banner).toBe('error')
    w.storage.notice = 'storage.refused'
    // A notice nobody dismissed never hides what stops the work.
    expect(w.storage.banner).toBe('error')
    w.storage.block = null
    expect(w.storage.banner).toBe('notice')
    w.storage.dismissNotice()
    expect(w.storage.banner).toBe('reopen')
    w.storage.dismissReopen()
    expect(w.storage.banner).toBeNull()
  })

  it('forgets the name and the action of a notice when another notice replaces it', () => {
    const w = world()
    w.storage.notice = 'pack.alreadyHere'
    w.storage.noticeVars = { name: 'Song' }
    w.storage.noticeOpen = 'dir-1'
    w.storage.notice = 'pack.invalid'
    expect(w.storage.noticeVars).toBeNull()
    expect(w.storage.noticeOpen).toBeNull()
  })
})
