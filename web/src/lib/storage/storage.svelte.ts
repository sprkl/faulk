// The project's life in a real folder (SV-2): which folder the project is saved into, when it is
// written, what the interface says about it, and what happens when the browser withdraws the right
// to write, the folder is edited from outside, or the page is about to go.
//
// The core decides *what* to write (`ProjectCore.save`); `ProjectStore` writes it, in a Worker.
// This controller decides *when*, in what order, and what to tell the user. Every operation that
// changes which folder the project is in, or writes it, goes through one queue, so a save never
// overlaps `project.new`, an opening or a save as.
import type { MessageKey } from '../i18n'
import type { FolderEntry, SaveFrame, SaveMode } from '../project/core'
import type { OpenOutcome } from '../stores/open'
import {
  StoreError,
  type FolderHandle,
  type FolderPicker,
  type ProjectStore,
  type RecentProject,
  type RecentProjects,
  type Signature,
} from './types'

/** Waits this long after the last change before writing (a knob drag is one write, not hundreds). */
export const AUTOSAVE_DELAY_MS = 1500
/** Under continuous editing, writes at least this often. */
export const AUTOSAVE_MAX_WAIT_MS = 10_000
/** A retry after a failed write, for a failure the user cannot fix with a click (a full disk, a locked file). */
export const RETRY_MS = 10_000
/** Waits this long after the folder was seen changing, so a burst of events is one check. */
const EXTERNAL_DELAY_MS = 400

/** What the controller needs of the page's project: the dispatcher implements it. */
export interface ProjectHost {
  /** Whether the project has changes its folder does not hold; `null` while there is no core yet. */
  dirty(): boolean | null
  /** The writes for the folder (see `ProjectCore.save`); `null` while there is no core. */
  save(mode: SaveMode): SaveFrame | null
  /** The folder holds what the last `save` gave, or does not. */
  saved(applied: boolean): void
  /** `project.open` on the folder's files. */
  openProject(name: string, entries: FolderEntry[]): Promise<OpenOutcome>
  /** `project.new`; false when the project cannot be replaced now. */
  newProject(): Promise<boolean>
  /** The identity of the project now. */
  projectId(): string | null
}

/** The browser calls the controller makes, so tests run it without a dialog, a Worker or a clock. */
export interface StorageDeps {
  picker: FolderPicker
  recent: RecentProjects
  openStore(handle: FolderHandle): Promise<ProjectStore>
  permission(handle: FolderHandle): Promise<PermissionState>
  askPermission(handle: FolderHandle): Promise<PermissionState>
  /** Holds the right to write the project; `null` when another tab holds it. */
  lock(id: string): Promise<(() => void) | null>
  /** Calls back when the folder may have changed; returns what stops watching. */
  watch(handle: FolderHandle, onChange: () => void): () => void
  now(): number
}

/** What stands between the user and a write: the interface shows a banner for each. */
export type Block = 'permission' | 'error' | 'external'

/** What the top bar shows. */
export type SaveStatus =
  /** No folder: the project lives in this tab only. */
  | 'unsaved'
  /** Changes wait for the next write. */
  | 'pending'
  | 'saving'
  | 'saved'
  /** The browser withdrew the right to write; changes wait for a click. */
  | 'paused'
  | 'failed'
  /** The folder was edited from outside while changes were unsaved. */
  | 'conflict'

/** Which action a confirmation guards. */
export type Discard = 'new' | 'open'

interface Attached {
  handle: FolderHandle
  store: ProjectStore
  release: () => void
  /** The stamps the folder should have if only we wrote it. */
  expected: Signature
  stopWatching: () => void
}

/**
 * How long to wait after the last change, given what the last write cost the main thread
 * (`coreMs`): the base delay, stretched so a write takes about 3 % of the thread at most.
 */
export function autosaveDelay(coreMs: number): number {
  return Math.min(60_000, Math.max(AUTOSAVE_DELAY_MS, 30 * coreMs))
}

/** The identity in a folder's `project.json`, to lock it before the project is replaced. */
export function projectIdOf(entries: FolderEntry[]): string | null {
  const file = entries.find((entry) => entry.path === 'project.json')
  if (!file || !('bytes' in file)) return null
  try {
    const id = (JSON.parse(new TextDecoder().decode(file.bytes)) as { id?: unknown }).id
    return typeof id === 'string' ? id : null
  } catch {
    return null
  }
}

/** Paths whose stamps differ between what the folder should hold and what it holds. */
export function changedPaths(expected: Signature, found: Signature): string[] {
  return Object.keys({ ...expected, ...found }).filter((path) => expected[path] !== found[path])
}

export class ProjectStorage {
  /** The folder the project is saved into; `null` until one is chosen. */
  folder = $state.raw<{ name: string } | null>(null)
  /** The project has changes the folder does not hold. */
  dirty = $state(false)
  saving = $state(false)
  /** When the folder last received a write from this page, in ms. */
  savedAt = $state<number | null>(null)
  block = $state<Block | null>(null)
  /** Text of the last failure, for a developer. */
  errorDetail = $state<string | null>(null)
  /** The project that was open last time, waiting for a click (the browser asks for the permission again). */
  reopen = $state.raw<{ id: string; name: string } | null>(null)
  recent = $state.raw<RecentProject[]>([])
  /** Something the user should read: why a folder was not taken, why a save did not happen. */
  notice = $state<MessageKey | null>(null)
  /** A question the interface is asking (New or Open on a project whose changes cannot be kept). */
  confirm = $state.raw<{ action: Discard; name: string | null } | null>(null)

  /**
   * What the last write cost: `coreMs` is time on the page's main thread (the core computing the
   * write), `writeMs` the time until the store, in its Worker, had applied it. For measurements.
   */
  lastWrite: { coreMs: number; writeMs: number; files: number; bytes: number } | null = null

  private current: Attached | null = null
  private queue: Promise<unknown> = Promise.resolve()
  private debounce: ReturnType<typeof setTimeout> | null = null
  private maxWait: ReturnType<typeof setTimeout> | null = null
  private retry: ReturnType<typeof setTimeout> | null = null
  private externalTimer: ReturnType<typeof setTimeout> | null = null
  private answer: ((discard: boolean) => void) | null = null
  /** The folder changed while a write was under way: look again once it is done. */
  private changedWhileSaving = false

  constructor(
    private readonly host: ProjectHost,
    private readonly deps: StorageDeps,
  ) {}

  /** Whether this browser can keep a project in a folder at all. */
  get supported(): boolean {
    return this.deps.picker.supported
  }

  /** Whether leaving the page now would lose work: changes that no folder holds yet. */
  get unsaved(): boolean {
    return this.dirty || this.saving
  }

  get status(): SaveStatus {
    if (this.block === 'external') return 'conflict'
    if (this.block === 'permission') return 'paused'
    if (this.saving) return 'saving'
    if (this.block === 'error') return 'failed'
    if (!this.folder) return 'unsaved'
    return this.dirty ? 'pending' : 'saved'
  }

  // ------------------------------------------------------------------ changes and autosave

  /** The dispatcher calls this after every command and every opening: the project may have changed. */
  changed() {
    const dirty = this.host.dirty()
    if (dirty === null) return
    this.dirty = dirty
    if (dirty && this.current && this.block !== 'permission' && this.block !== 'external') this.schedule()
  }

  private schedule() {
    // The core computes a write on the page's main thread (serializing and hashing the project): on
    // a big project that is felt. So the more a write costs, the less often it runs: at most about
    // one thirtieth of the main thread, never less often than once a minute.
    const delay = autosaveDelay(this.lastWrite?.coreMs ?? 0)
    if (this.debounce) clearTimeout(this.debounce)
    this.debounce = setTimeout(() => void this.autosave(), delay)
    this.maxWait ??= setTimeout(() => void this.autosave(), Math.max(AUTOSAVE_MAX_WAIT_MS, 2 * delay))
  }

  private cancelTimers() {
    for (const timer of [this.debounce, this.maxWait, this.retry]) if (timer) clearTimeout(timer)
    this.debounce = this.maxWait = this.retry = null
  }

  private autosave(): Promise<unknown> {
    this.cancelTimers()
    return this.enqueue(() => this.write('changed'))
  }

  /** Runs `task` after everything queued before it, whatever came of that. */
  private enqueue<T>(task: () => Promise<T>): Promise<T> {
    const run = this.queue.then(task, task)
    this.queue = run.catch(() => undefined)
    return run
  }

  /**
   * One write of the project into its folder. Returns whether the folder holds the project
   * afterwards. Never throws: a failure becomes a block, and the changes stay in memory.
   */
  private async write(mode: SaveMode): Promise<boolean> {
    const current = this.current
    if (!current) return false
    if (this.block === 'permission' || this.block === 'external') return false
    this.cancelTimers()
    if ((await this.deps.permission(current.handle)) !== 'granted') {
      this.block = 'permission'
      return false
    }
    // The folder may have been replaced while the permission was asked.
    if (this.current !== current) return false
    let frame: SaveFrame | null
    const startedComputing = performance.now()
    try {
      frame = this.host.save(mode)
    } catch (error) {
      console.error(error)
      return this.fail(current, error)
    }
    if (!frame) return false
    // What the core spent on the page's main thread to compute this write (wasm: serialize, hash, copy out).
    const coreMs = performance.now() - startedComputing
    if (frame.status === 'failed') {
      this.host.saved(false)
      return this.fail(current, new Error(frame.message))
    }
    if (frame.entries.length === 0) {
      this.host.saved(true)
      // A save the user asked for says it was done, though the folder already held everything.
      if (mode !== 'changed') this.savedAt = this.deps.now()
      this.finish(current)
      return true
    }
    this.saving = true
    try {
      // Counted before the store takes the buffers (they are handed to its Worker, not copied).
      const bytes = frame.entries.reduce((n, e) => n + (e.type === 'write' ? e.bytes.length : 0), 0)
      const files = frame.entries.length
      const startedWriting = performance.now()
      const written = await current.store.apply(frame.entries)
      this.lastWrite = { coreMs, writeMs: performance.now() - startedWriting, files, bytes }
      if (this.current !== current) return false
      this.host.saved(true)
      Object.assign(current.expected, written)
      for (const entry of frame.entries) if (entry.type === 'remove') delete current.expected[entry.path]
      this.savedAt = this.deps.now()
      this.block = null
      this.errorDetail = null
    } catch (error) {
      this.host.saved(false)
      this.saving = false
      return this.fail(current, error)
    }
    this.saving = false
    this.finish(current)
    return true
  }

  /** After a write: edits made meanwhile are still to write; a folder seen changing meanwhile is looked at. */
  private finish(current: Attached) {
    if (this.current !== current) return
    this.dirty = this.host.dirty() ?? false
    if (this.dirty) this.schedule()
    if (this.changedWhileSaving) {
      this.changedWhileSaving = false
      this.folderMayHaveChanged()
    }
  }

  private fail(current: Attached, error: unknown): boolean {
    if (this.current !== current) return false
    this.dirty = this.host.dirty() ?? true
    this.errorDetail = error instanceof Error ? error.message : String(error)
    if (error instanceof StoreError && error.code === 'permission') {
      this.block = 'permission'
    } else {
      this.block = 'error'
      // A full disk or a locked file may clear by itself: try again later.
      this.retry ??= setTimeout(() => {
        this.retry = null
        void this.autosave()
      }, RETRY_MS)
    }
    return false
  }

  /** The user asked to save now (the menu, Ctrl+S). A project without a folder asks for one. */
  save(): Promise<boolean> {
    if (!this.current) return this.saveAs()
    this.cancelTimers()
    // Forced: it looks at the session too, and a user who presses Save wants the folder checked.
    return this.enqueue(() => this.write('forced'))
  }

  /** Writes the changes and says whether the folder is up to date. Used before leaving the project. */
  flush(): Promise<boolean> {
    this.cancelTimers()
    return this.enqueue(async () => {
      if (!this.current) return !this.dirty
      if (!(this.host.dirty() ?? false)) return true
      return this.write('changed')
    })
  }

  // ------------------------------------------------------------------ choosing a folder

  /**
   * Save as: the user picks an empty folder and the project is written into it whole, with a
   * new identity when it already had a folder. Call straight from a click.
   */
  async saveAs(): Promise<boolean> {
    if (!this.supported) {
      this.notice = 'storage.unsupported'
      return false
    }
    const picked = await this.deps.picker.pick()
    if (picked.status === 'cancelled') return false
    if (picked.status === 'refused') {
      this.notice = 'storage.refused'
      return false
    }
    this.notice = null
    return this.enqueue(() => this.saveInto(picked.handle))
  }

  private async saveInto(handle: FolderHandle): Promise<boolean> {
    this.cancelTimers()
    let store: ProjectStore | null = null
    let release: (() => void) | null = null
    let frame: SaveFrame | null = null
    try {
      store = await this.deps.openStore(handle)
      if (!(await store.isEmpty())) {
        this.notice = 'storage.notEmpty'
        store.close()
        return false
      }
      const startedComputing = performance.now()
      frame = this.host.save(this.current ? 'copy' : 'everything')
      const coreMs = performance.now() - startedComputing
      if (!frame) {
        store.close()
        return false
      }
      if (frame.status === 'failed') throw new Error(frame.message)
      const id = this.host.projectId()
      release = id ? await this.deps.lock(id) : () => undefined
      if (!release) {
        this.notice = 'storage.locked'
        this.host.saved(false)
        store.close()
        return false
      }
      // Blobs first, like any save: `project.json` is the last file and the commit.
      if (this.current) await store.copyBlobsFrom(this.current.store)
      // Counted before the store takes the buffers (they are handed to its Worker, not copied).
      const bytes = frame.entries.reduce((n, e) => n + (e.type === 'write' ? e.bytes.length : 0), 0)
      const files = frame.entries.length
      const startedWriting = performance.now()
      await store.apply(frame.entries)
      this.lastWrite = { coreMs, writeMs: performance.now() - startedWriting, files, bytes }
      this.host.saved(true)
    } catch (error) {
      if (frame) this.host.saved(false)
      release?.()
      store?.close()
      this.notice = error instanceof StoreError && error.code === 'permission' ? 'storage.refused' : 'storage.saveFailed'
      if (!(error instanceof StoreError)) console.error(error)
      return false
    }
    await this.attach(handle, store, release, true)
    return true
  }

  /** Open: the user picks a project folder. Call straight from a click. */
  async openFolder(): Promise<boolean> {
    if (!this.supported) {
      this.notice = 'storage.unsupported'
      return false
    }
    const picked = await this.deps.picker.pick()
    if (picked.status === 'cancelled') return false
    if (picked.status === 'refused') {
      this.notice = 'storage.refused'
      return false
    }
    return this.enqueue(() => this.openHandle(picked.handle))
  }

  /** Open a recent project. Call straight from a click: the browser may ask for the permission again. */
  async openRecent(id: string): Promise<boolean> {
    const entry = this.recent.find((recent) => recent.id === id)
    if (!entry) return false
    if ((await this.deps.askPermission(entry.handle)) !== 'granted') {
      this.notice = 'storage.permissionDenied'
      return false
    }
    this.reopen = null
    return this.enqueue(() => this.openHandle(entry.handle, id))
  }

  /** The banner after a reload: the project that was open comes back with the user's click. */
  reopenLast(): Promise<boolean> {
    const last = this.reopen
    return last ? this.openRecent(last.id) : Promise.resolve(false)
  }

  private async openHandle(handle: FolderHandle, forgetIfGone?: string): Promise<boolean> {
    if (!(await this.makeRoom('open'))) return false
    let store: ProjectStore | null = null
    let release: (() => void) | null = null
    try {
      store = await this.deps.openStore(handle)
      if (!(await store.hasProject())) {
        this.notice = 'project.open.notAProject'
        if (forgetIfGone) await this.deps.recent.forget(forgetIfGone)
        store.close()
        return false
      }
      const { entries } = await store.readFolder()
      const id = projectIdOf(entries)
      release = id ? await this.deps.lock(id) : () => undefined
      if (!release) {
        this.notice = 'storage.locked'
        store.close()
        return false
      }
      const outcome = await this.host.openProject(handle.name, entries)
      if (outcome.status === 'refused') {
        release()
        store.close()
        return false
      }
    } catch (error) {
      release?.()
      store?.close()
      this.notice = error instanceof StoreError && error.code === 'permission' ? 'storage.permissionDenied' : 'project.open.unreadable'
      if (!(error instanceof StoreError)) console.error(error)
      return false
    }
    this.notice = null
    await this.attach(handle, store, release, false)
    return true
  }

  /** New project: asks first when changes would be lost. */
  newProject(): Promise<boolean> {
    return this.enqueue(async () => {
      if (!(await this.makeRoom('new'))) return false
      if (!(await this.host.newProject())) return false
      await this.detach()
      this.reset()
      return true
    })
  }

  /**
   * The project is about to be replaced. With a folder, the changes are written first and the
   * user is asked only if they could not be. Without one, the user is asked whenever the
   * project was changed. True when it may go ahead.
   */
  private async makeRoom(action: Discard): Promise<boolean> {
    if (this.current && (this.host.dirty() ?? false)) await this.write('changed')
    this.dirty = this.host.dirty() ?? false
    if (!this.dirty) return true
    return this.ask(action)
  }

  private ask(action: Discard): Promise<boolean> {
    return new Promise((resolve) => {
      this.answer = resolve
      this.confirm = { action, name: this.folder?.name ?? null }
    })
  }

  /** The user's answer to `confirm`: discard the changes and go ahead, or cancel. */
  respond(discard: boolean) {
    const answer = this.answer
    this.answer = null
    this.confirm = null
    answer?.(discard)
  }

  // ------------------------------------------------------------------ the folder the project is in

  private async attach(handle: FolderHandle, store: ProjectStore, release: () => void, wrote: boolean) {
    await this.detach()
    const current: Attached = { handle, store, release, expected: {}, stopWatching: () => undefined }
    this.current = current
    this.folder = { name: handle.name }
    this.block = null
    this.errorDetail = null
    this.dirty = this.host.dirty() ?? false
    if (wrote) this.savedAt = this.deps.now()
    else this.savedAt = null
    await store.cleanUp()
    try {
      current.expected = await store.scan()
    } catch {
      // The first check will tell.
    }
    if (this.current !== current) return
    current.stopWatching = this.deps.watch(handle, () => this.folderMayHaveChanged())
    await this.deps.recent.remember(handle)
    this.recent = await this.deps.recent.list()
    if (this.dirty) this.schedule()
  }

  private async detach() {
    const current = this.current
    this.cancelTimers()
    if (this.externalTimer) clearTimeout(this.externalTimer)
    this.externalTimer = null
    this.current = null
    if (!current) return
    current.stopWatching()
    current.release()
    current.store.close()
  }

  private reset() {
    this.folder = null
    this.dirty = false
    this.saving = false
    this.savedAt = null
    this.block = null
    this.errorDetail = null
    this.notice = null
  }

  // ------------------------------------------------------------------ the browser takes the right away

  /** The banner's button: ask for the permission again (a prompt), then write what waited. */
  async resume(): Promise<boolean> {
    const current = this.current
    if (!current) return false
    if ((await this.deps.askPermission(current.handle)) !== 'granted') return false
    if (this.block === 'permission') this.block = null
    return this.autosave().then(() => !this.dirty)
  }

  /** The page became visible or focused: a permission that came back, or a folder edited meanwhile. */
  async wake() {
    const current = this.current
    if (!current) return
    if (this.block === 'permission' && (await this.deps.permission(current.handle)) === 'granted') {
      this.block = null
      void this.autosave()
      return
    }
    this.folderMayHaveChanged()
  }

  /** The page is going to the background: write what waits. */
  sleep() {
    if (this.current && this.dirty && this.block !== 'permission' && this.block !== 'external') void this.autosave()
  }

  // ------------------------------------------------------------------ the folder is edited from outside

  private folderMayHaveChanged() {
    if (!this.current) return
    if (this.saving) {
      this.changedWhileSaving = true
      return
    }
    if (this.externalTimer) clearTimeout(this.externalTimer)
    this.externalTimer = setTimeout(() => {
      this.externalTimer = null
      void this.enqueue(() => this.checkExternal())
    }, EXTERNAL_DELAY_MS)
  }

  private async checkExternal() {
    const current = this.current
    if (!current || this.block === 'external') return
    let found: Signature
    try {
      found = await current.store.scan()
    } catch {
      return
    }
    if (this.current !== current || this.saving) return
    if (changedPaths(current.expected, found).length === 0) return
    if (this.host.dirty() ?? false) {
      // The user's version and the disk's differ: the user chooses.
      this.block = 'external'
      this.cancelTimers()
      return
    }
    await this.reload('storage.reloaded')
  }

  /** Reads the folder again and replaces the project with it. */
  private async reload(notice: MessageKey | null): Promise<boolean> {
    const current = this.current
    if (!current) return false
    try {
      const { entries } = await current.store.readFolder()
      const outcome = await this.host.openProject(current.handle.name, entries)
      if (outcome.status === 'refused') {
        // The folder cannot be read as a project now (a checkout half done, a deleted project.json):
        // no write goes into it until the user chooses.
        this.block = 'external'
        this.cancelTimers()
        return false
      }
      current.expected = await current.store.scan()
      this.block = null
      this.dirty = this.host.dirty() ?? false
      this.notice = notice
      return true
    } catch (error) {
      console.error(error)
      return false
    }
  }

  /** The conflict banner: keep the project as it is; the next write replaces the folder's files. */
  keepMine(): Promise<boolean> {
    return this.enqueue(async () => {
      const current = this.current
      if (!current) return false
      this.block = null
      return this.write('rewrite')
    })
  }

  /** The conflict banner: take the folder's version and drop the changes made since the last write. */
  loadDisk(): Promise<boolean> {
    return this.enqueue(() => this.reload('storage.reloaded'))
  }

  /** A command replaced the project behind this controller's back (`project.new`): it has no folder now. */
  forgetFolder() {
    void this.enqueue(async () => {
      await this.detach()
      this.reset()
    })
  }

  /** The user does not want the project of last time back. */
  dismissReopen() {
    this.reopen = null
  }

  /** Says the banner for a notice was read. */
  dismissNotice() {
    this.notice = null
  }

  // ------------------------------------------------------------------ the start of a page

  /**
   * On a fresh page: the project that was open last time comes back. With the permission still
   * granted it opens at once; otherwise a banner offers it, because the browser wants a click
   * before it gives the folder back.
   */
  async restore() {
    this.recent = await this.deps.recent.list()
    const last = this.recent[0]
    if (!last || this.current || (this.host.dirty() ?? false)) return
    if ((await this.deps.permission(last.handle)) === 'granted') {
      await this.enqueue(() => this.openHandle(last.handle, last.id))
    } else {
      this.reopen = { id: last.id, name: last.name }
    }
  }
}
