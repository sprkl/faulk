// The project's life in a real folder (SV-2): which folder the project is saved into, when it is
// written, what the interface says about it, and what happens when the browser withdraws the right
// to write, the folder is edited from outside, or the page is about to go.
//
// The core decides *what* to write (`ProjectCore.save`); `ProjectStore` writes it, in a Worker.
// This controller decides *when*, in what order, and what to tell the user. Every operation that
// changes which folder the project is in, or writes it, goes through one queue, so a save never
// overlaps `project.new`, an opening or a save as.
//
// SV-3 adds the browser's own storage (the OPFS): a project with no folder is kept there as a
// draft, written as often as a folder would be and offered again when the page reopens; where the
// browser has no folder to offer (Firefox, Safari) it is the project's normal home. A project in
// the OPFS is "browser" in this file, a project in a folder the user chose is "folder".
import type { MessageKey } from '../i18n'
import { needsBytes, type FolderEntry, type SaveEntry, type SaveFrame, type SaveMode } from '../project/core'
import type { PackExport, PackImport } from '../project/core'
import type { OpenOutcome } from '../stores/open'
import {
  StoreError,
  type BrowserProject,
  type BrowserProjects,
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
  /** Whether the project itself, not only its session (selection, solo...), has changes its folder does not hold. */
  projectDirty(): boolean | null
  /** The writes for the folder (see `ProjectCore.save`); `null` while there is no core. */
  save(mode: SaveMode): SaveFrame | null
  /** The folder holds what the last `save` gave, or does not. */
  saved(applied: boolean): void
  /** `project.open` on the folder's files. */
  openProject(name: string, entries: FolderEntry[]): Promise<OpenOutcome>
  /** `project.new`, named `name` (else the page's draft name); false when the project cannot be replaced now. */
  newProject(name?: string): Promise<boolean>
  /** The identity of the project now. */
  projectId(): string | null
  /** The name of the project now (for the file an export hands over). */
  projectName(): string | null
  /** The `.faulkpack` of a project folder given whole (see `ProjectCore.packExport`). */
  packExport(files: { path: string; bytes: Uint8Array }[]): Promise<PackExport>
  /** The files of a `.faulkpack`, validated (see `ProjectCore.packImport`). */
  packImport(zip: Uint8Array): Promise<PackImport>
  /** Makes the project core ahead of a click that will need it, so the click still has its permission to prompt. */
  prepare?(): Promise<void>
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
  /** The browser's own storage (the OPFS), where drafts and browser projects live. */
  browser: BrowserProjects
  /** Asks the user for a `.faulkpack` (a file chooser; call straight from a click); `null` when cancelled. */
  pickPack(): Promise<File | null>
  /** Hands a file to the user (a download). */
  download(name: string, blob: Blob): void
}

/** The biggest `.faulkpack` the page reads: the core holds the pack and its files in memory (`crates/project-wasm` `pack.rs`). */
export const MAX_PACK_BYTES = 192 << 20

/** What stands between the user and a write: the interface shows a banner for each. */
export type Block =
  | 'permission'
  | 'error'
  | 'external'
  /** The browser's storage is full: the changes stay in the tab until room is made. */
  | 'full'

/** Where the project is kept. */
export type Place = 'none' | 'folder' | 'browser'

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

/**
 * Which action a confirmation guards. The page asks whoever started the action (a click or an agent's
 * command): `delete` a browser project, and the two ways out of a conflict, `keepMine` (the folder's
 * files are replaced by the project) and `loadFolder` (the project is replaced by the folder's files).
 */
export type Discard = 'new' | 'open' | 'delete' | 'keepMine' | 'loadFolder'

interface Attached {
  handle: FolderHandle
  store: ProjectStore
  release: () => void
  /** The stamps the folder should have if only we wrote it. */
  expected: Signature
  stopWatching: () => void
  /** For a browser project, the lock on its directory (see `dirLock`). */
  releaseDir: (() => void) | null
  kind: 'folder' | 'browser'
  /** For a browser project, its directory in the OPFS. */
  dir: string | null
}

/** The lock that says a tab is making or using the OPFS directory `dir`: another tab's sweep leaves it alone. */
const dirLock = (dir: string) => `dir.${dir}`

/**
 * How long to wait after the last change, given what the last write cost the main thread
 * (`coreMs`): the base delay, stretched so a write takes about 3 % of the thread at most.
 */
export function autosaveDelay(coreMs: number): number {
  return Math.min(60_000, Math.max(AUTOSAVE_DELAY_MS, 30 * coreMs))
}

/** The identity in a folder's `project.json`, to lock it before the project is replaced. */
export function projectIdOf(entries: readonly { path: string; bytes?: Uint8Array; size?: number }[]): string | null {
  const file = entries.find((entry) => entry.path === 'project.json')
  if (!file?.bytes) return null
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
  /**
   * The project that was open last time, waiting for a click: a folder (the browser asks for the
   * permission again) or a draft in the browser's storage.
   */
  reopen = $state.raw<{ id: string; name: string; kind: 'folder' | 'browser' } | null>(null)
  recent = $state.raw<RecentProject[]>([])
  /** The project is kept in the browser's storage (a draft, or the project itself where there is no folder). */
  inBrowser = $state(false)
  /** The projects kept in the browser's storage, newest first (the menu lists them). */
  browserProjects = $state.raw<BrowserProject[]>([])
  /** What `navigator.storage.persist()` answered: `null` until asked. */
  persisted = $state<boolean | null>(null)
  /** The warning "your projects are in the browser; export them" was read this session. */
  private browserWarned = $state(false)
  /** Something the user should read: why a folder was not taken, why a save did not happen. */
  private shownNotice = $state<MessageKey | null>(null)
  /** What the notice names (a project), and the project of the browser it can open for the user (#474). Cleared with every new notice. */
  noticeVars = $state.raw<{ name: string } | null>(null)
  noticeOpen = $state.raw<string | null>(null)
  get notice(): MessageKey | null {
    return this.shownNotice
  }
  set notice(key: MessageKey | null) {
    this.shownNotice = key
    this.noticeVars = null
    this.noticeOpen = null
  }
  /** A question the interface is asking (New or Open on a project whose changes cannot be kept). */
  confirm = $state.raw<{ action: Discard; name: string | null } | null>(null)

  /**
   * What the last write cost: `coreMs` is time on the page's main thread (the core computing the
   * write), `writeMs` the time until the store, in its Worker, had applied it. For measurements.
   */
  lastWrite: { coreMs: number; writeMs: number; files: number; bytes: number } | null = null

  private current: Attached | null = null
  private queue: Promise<unknown> = Promise.resolve()
  /** Set while a project is being put in its place (see `attach`); `current` has a gap meanwhile. */
  private attaching: Promise<void> | null = null
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

  /** The folder the project is saved into, for a file dialog that opens there (the export). */
  async folderHandle(): Promise<FolderHandle | null> {
    // Not while an attach has no project for a moment.
    while (this.attaching) await this.attaching
    return this.current?.handle ?? null
  }

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
    if (this.block === 'error' || this.block === 'full') return 'failed'
    if (!this.folder && !this.inBrowser) {
      // A change waits for its first write into the browser's storage.
      return this.dirty && this.deps.browser.available ? 'pending' : 'unsaved'
    }
    return this.dirty ? 'pending' : 'saved'
  }

  /** Where the project is kept now. */
  get place(): Place {
    return this.folder ? 'folder' : this.inBrowser ? 'browser' : 'none'
  }

  /** Whether this browser keeps projects in its own storage (drafts, or the projects themselves). */
  get browserAvailable(): boolean {
    return this.deps.browser.available
  }

  /**
   * The warning where the browser's storage is the only home (no folder to choose): "your projects
   * are in the browser; export them". `volatile` when the browser would not promise to keep it.
   */
  get browserWarning(): 'export' | 'volatile' | null {
    if (this.supported || !this.inBrowser || this.browserWarned) return null
    return this.persisted === false ? 'volatile' : 'export'
  }

  dismissBrowserWarning() {
    this.browserWarned = true
  }

  // ------------------------------------------------------------------ changes and autosave

  /** The dispatcher calls this after every command and every opening: the project may have changed. */
  changed() {
    if (this.host.dirty() === null) return
    const dirty = this.worthKeeping()
    this.dirty = dirty
    if (!dirty) return
    // With no home yet, the first write is the draft in the browser's storage (where there is one).
    const writable = this.current ? this.block !== 'permission' && this.block !== 'external' : this.deps.browser.available
    if (writable) this.schedule()
  }

  /**
   * Whether the project has changes that deserve a write or a warning. Into a home it has, every
   * change does (the selection, a solo, are saved with the project). With none, only a change to
   * the project itself does: a page that was only opened, and selected its first track, has no
   * draft to make and nothing to lose.
   */
  private worthKeeping(): boolean {
    if (this.current) return this.host.dirty() ?? false
    return this.host.projectDirty() ?? this.host.dirty() ?? false
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
    return this.enqueue(() => (this.current ? this.write('changed') : this.startBrowserProject()))
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
    // The browser's own storage is not asked for a permission: it is the page's.
    if (current.kind === 'folder' && (await this.deps.permission(current.handle)) !== 'granted') {
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
    if (mode === 'rewrite') {
      // The folder's version is being replaced: what it holds of the tracks that this project no longer
      // has (a track deleted since, a master lane deleted since, a file an outside edit added) goes too, or it would come back on opening.
      try {
        const written = frame.entries.map((entry) => entry.path)
        for (const path of Object.keys(await current.store.scan())) {
          if ((path.startsWith('tracks/') || path.startsWith('master/')) && !written.includes(path)) frame.entries.push({ type: 'remove', path })
        }
      } catch (error) {
        this.host.saved(false)
        return this.fail(current, error)
      }
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
      // Some of the files may have been written before it failed: they are ours, not an outside change.
      current.expected = await current.store.scan().catch(() => current.expected)
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
      this.block = error instanceof StoreError && error.code === 'full' && current.kind === 'browser' ? 'full' : 'error'
      // A full disk or a locked file may clear by itself: try again later.
      this.retry ??= setTimeout(() => {
        this.retry = null
        void this.autosave()
      }, RETRY_MS)
    }
    return false
  }

  /** The user asked to save now (the menu, Ctrl+S). A project without a folder asks for one. */
  async save(): Promise<boolean> {
    // After an attach under way: it has no project for a moment, and a decision read then would send the
    // user to a second folder dialog or write nowhere. No await from here on, so the decision and the
    // folder dialog still come straight from the click.
    while (this.attaching) await this.attaching
    // Where the browser has no folder to offer, the browser's storage is the home: Save keeps the project there.
    // Also the banner's "Try again" when the first write into it failed.
    if (!this.current && this.deps.browser.available && (!this.supported || this.block)) {
      this.cancelTimers()
      return this.enqueue(() => this.startBrowserProject(true))
    }
    // A draft is not a place to leave a project in: where a folder can be chosen, saving asks for one
    // (unless a write into the draft failed, and the user is asking to try again).
    if (!this.current || (this.current.kind === 'browser' && this.supported && !this.block)) return this.saveAs()
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
    // The chooser opens only from a click: an agent without one is told so, and nothing is chosen.
    if (this.needsGesture()) return false
    const picked = await this.deps.picker.pick()
    if (picked.status === 'cancelled') return false
    if (picked.status === 'refused') {
      this.notice = 'storage.refused'
      return false
    }
    this.notice = null
    return this.enqueue(() => this.saveInto({ kind: 'folder', handle: picked.handle }))
  }

  /**
   * The first write of a project with no home into the browser's storage: the draft (or, where
   * there is no folder to choose, the project's home). `force` writes a project that has no
   * change yet (Save, in a browser with no folder).
   */
  private async startBrowserProject(force = false): Promise<boolean> {
    if (this.current || !this.deps.browser.available) return false
    if (!force && !this.worthKeeping()) return false
    return this.saveInto({ kind: 'browser' })
  }

  /**
   * Writes the whole project into a place that holds nothing yet, and makes it the project's home:
   * an empty folder the user chose, or a new directory of the browser's storage. From a folder it is a
   * copy with a new identity; from a project in the browser it is the same project moving (blobs
   * come along, the lock on its identity is carried over, and the draft is deleted once the folder
   * holds everything). A failure leaves the project where it was.
   */
  private saveInto(target: { kind: 'folder'; handle: FolderHandle } | { kind: 'browser' }): Promise<boolean> {
    // The whole move is one gap for Save and the export folder: the blobs are copied and the frame written
    // while the old home is still current, and a Save then would ask for a second folder.
    return this.holdingBarrier(this.writeInto(target))
  }

  private async writeInto(target: { kind: 'folder'; handle: FolderHandle } | { kind: 'browser' }): Promise<boolean> {
    this.cancelTimers()
    const from = this.current
    const promotedFrom = from?.kind === 'browser' && target.kind === 'folder' ? from : null
    let store: ProjectStore | null = null
    let release: (() => void) | null = null
    let releaseDir: (() => void) | null = null
    let dir: string | null = null
    let frame: SaveFrame | null = null
    let handle: FolderHandle
    try {
      if (target.kind === 'browser') {
        // The directory's lock is taken before the directory exists, so another tab's sweep of
        // half-made directories can never see this one as abandoned.
        dir = crypto.randomUUID()
        releaseDir = await this.deps.lock(dirLock(dir))
        handle = await this.deps.browser.create(dir)
      } else {
        handle = target.handle
      }
      store = await this.deps.openStore(handle)
      if (!(await store.isEmpty())) {
        this.notice = 'storage.notEmpty'
        return await this.abandon(null, store, null, releaseDir, dir)
      }
      const startedComputing = performance.now()
      frame = this.host.save(from?.kind === 'folder' ? 'copy' : 'everything')
      const coreMs = performance.now() - startedComputing
      if (!frame) return await this.abandon(null, store, null, releaseDir, dir)
      if (frame.status === 'failed') throw new Error(frame.message)
      if (promotedFrom) {
        // Same project, same identity: its lock moves to the folder instead of being taken twice.
        release = promotedFrom.release
      } else {
        const id = this.host.projectId()
        release = id ? await this.deps.lock(id) : () => undefined
        if (!release) {
          this.notice = 'storage.locked'
          return await this.abandon(frame, store, null, releaseDir, dir)
        }
      }
      // Blobs first, like any save: `project.json` is the last file and the commit.
      if (from) await store.copyBlobsFrom(from.store)
      // Counted before the store takes the buffers (they are handed to its Worker, not copied).
      const bytes = frame.entries.reduce((n, e) => n + (e.type === 'write' ? e.bytes.length : 0), 0)
      const files = frame.entries.length
      const startedWriting = performance.now()
      await store.apply(frame.entries)
      this.lastWrite = { coreMs, writeMs: performance.now() - startedWriting, files, bytes }
      this.host.saved(true)
    } catch (error) {
      await this.abandon(frame, store, promotedFrom ? null : release, releaseDir, dir)
      if (!(error instanceof StoreError)) console.error(error)
      if (target.kind === 'browser') {
        this.failBrowserStart(error)
        return false
      }
      this.notice = error instanceof StoreError && error.code === 'permission' ? 'storage.refused' : 'storage.saveFailed'
      return false
    }
    // The draft has been taken: the folder holds the project now, so the draft goes (inside `attach`,
    // before the page says "saved").
    const placed = { kind: target.kind, dir, releaseDir, discard: promotedFrom?.dir ?? null }
    if (promotedFrom) promotedFrom.release = () => undefined
    await this.attach(handle, store, release, true, placed)
    if (target.kind === 'browser') this.persistOnce()
    await this.refreshBrowserProjects()
    return true
  }

  /** A write into a new place did not happen: gives back what it took, and removes the directory it made. Returns false. */
  private async abandon(
    frame: SaveFrame | null,
    store: ProjectStore | null,
    release: (() => void) | null,
    releaseDir: (() => void) | null,
    dir: string | null,
  ): Promise<false> {
    if (frame) this.host.saved(false)
    release?.()
    store?.close()
    if (dir) await this.deps.browser.remove(dir).catch(() => undefined)
    releaseDir?.()
    return false
  }

  /** The first write into the browser's storage failed: said by a banner, tried again later. */
  private failBrowserStart(error: unknown) {
    // The storage turned out not to work at all (a private window): the project stays in the tab, as it did before drafts.
    if (!this.deps.browser.available) return
    this.dirty = this.host.dirty() ?? true
    this.errorDetail = error instanceof Error ? error.message : String(error)
    this.block = error instanceof StoreError && error.code === 'full' ? 'full' : 'error'
    this.retry ??= setTimeout(() => {
      this.retry = null
      void this.autosave()
    }, RETRY_MS)
  }

  /** Asks the browser to keep its storage, once. */
  private persistOnce() {
    if (this.persisted !== null) return
    void this.deps.browser.persist().then((kept) => {
      this.persisted = kept
    })
  }

  /** Reads the browser's projects again (the menu lists them). */
  async refreshBrowserProjects(): Promise<void> {
    if (!this.deps.browser.available) {
      this.browserProjects = []
      return
    }
    this.browserProjects = (await this.deps.browser.list()).projects
  }

  /**
   * Whether the browser would refuse a prompt or a file chooser now: they open only from a click. An
   * agent that runs the command is told so, instead of waiting for a dialog that never comes.
   */
  private needsGesture(): boolean {
    if (typeof navigator !== 'undefined' && navigator.userActivation && !navigator.userActivation.isActive) {
      this.notice = 'storage.needsClick'
      return true
    }
    return false
  }

  /** Reads the recent folders and the browser's projects again (`project.list_stored`; the menu lists them). */
  async refreshStored(): Promise<void> {
    this.recent = await this.deps.recent.list()
    await this.refreshBrowserProjects()
  }

  /** Open: the user picks a project folder. Call straight from a click. */
  async openFolder(): Promise<boolean> {
    if (!this.supported) {
      this.notice = 'storage.unsupported'
      return false
    }
    if (this.needsGesture()) return false
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
    // A list read before another tab forgot or added a folder: look again before saying it is unknown.
    let entry = this.recent.find((recent) => recent.id === id)
    if (!entry) {
      this.recent = await this.deps.recent.list()
      entry = this.recent.find((recent) => recent.id === id)
    }
    if (!entry) {
      this.notice = 'storage.recentGone'
      return false
    }
    // The prompt is needed only when the browser withdrew the permission, and it opens only from a click.
    let permission = await this.deps.permission(entry.handle)
    if (permission !== 'granted') {
      if (this.needsGesture()) return false
      permission = await this.deps.askPermission(entry.handle)
    }
    if (permission !== 'granted') {
      this.notice = 'storage.permissionDenied'
      return false
    }
    this.reopen = null
    return this.enqueue(() => this.openHandle(entry.handle, { forgetIfGone: id }))
  }

  /** The banner after a reload: the project that was open comes back with the user's click. */
  reopenLast(): Promise<boolean> {
    const last = this.reopen
    if (!last) {
      this.notice = 'storage.nothingToReopen'
      return Promise.resolve(false)
    }
    return last.kind === 'browser' ? this.openBrowserProject(last.id) : this.openRecent(last.id)
  }

  /**
   * Open a project kept in the browser's storage (a draft, or a project of a browser with no folder). Only one the
   * browser lists: a name that is not in its list is never handed to the storage.
   */
  openBrowserProject(dir: string): Promise<boolean> {
    return this.enqueue(async () => {
      await this.refreshBrowserProjects()
      if (!this.browserProjects.some((project) => project.dir === dir)) {
        if (this.reopen?.kind === 'browser' && this.reopen.id === dir) this.reopen = null
        this.notice = 'storage.browserGone'
        return false
      }
      this.reopen = null
      const handle = await this.deps.browser.open(dir)
      if (!handle) {
        this.notice = 'storage.browserGone'
        await this.refreshBrowserProjects()
        return false
      }
      const opened = await this.openHandle(handle, { kind: 'browser', dir })
      if (opened) this.persistOnce()
      return opened
    })
  }

  private async openHandle(handle: FolderHandle, options: { forgetIfGone?: string; kind?: 'folder' | 'browser'; dir?: string } = {}): Promise<boolean> {
    const kind = options.kind ?? 'folder'
    const dir = options.dir ?? null
    // The project that is open already: nothing to open, and its own lock must not refuse it.
    if (this.current && (kind === 'browser' ? this.current.dir === dir : await this.current.handle.isSameEntry?.(handle)?.catch(() => false))) return true
    if (!(await this.makeRoom('open'))) return false
    let store: ProjectStore | null = null
    let release: (() => void) | null = null
    let releaseDir: (() => void) | null = null
    try {
      if (dir) {
        releaseDir = await this.deps.lock(dirLock(dir))
        if (!releaseDir) {
          this.notice = 'storage.locked'
          return false
        }
      }
      store = await this.deps.openStore(handle)
      if (!(await store.hasProject())) {
        this.notice = 'project.open.notAProject'
        if (options.forgetIfGone) await this.deps.recent.forget(options.forgetIfGone)
        store.close()
        releaseDir?.()
        return false
      }
      const { entries } = await store.readFolder()
      const id = projectIdOf(entries)
      release = id ? await this.deps.lock(id) : () => undefined
      if (!release) {
        this.notice = 'storage.locked'
        store.close()
        releaseDir?.()
        return false
      }
      const outcome = await this.host.openProject(handle.name, entries)
      if (outcome.status === 'refused') {
        release()
        store.close()
        releaseDir?.()
        return false
      }
    } catch (error) {
      release?.()
      store?.close()
      releaseDir?.()
      this.notice = error instanceof StoreError && error.code === 'permission' ? 'storage.permissionDenied' : 'project.open.unreadable'
      if (!(error instanceof StoreError)) console.error(error)
      return false
    }
    this.notice = null
    await this.attach(handle, store, release, false, { kind, dir, releaseDir })
    return true
  }

  /** New project, named `name` (else the page's draft name): asks first when changes would be lost. */
  newProject(name?: string): Promise<boolean> {
    return this.enqueue(async () => {
      if (!(await this.makeRoom('new'))) return false
      if (!(await this.host.newProject(name))) return false
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
    // A change still waiting for its first write into the browser's storage is kept there now, not discarded.
    else if (!this.current && this.worthKeeping() && this.deps.browser.available) await this.startBrowserProject()
    this.dirty = this.worthKeeping()
    if (!this.dirty) return true
    return this.ask(action)
  }

  private ask(action: Discard, name: string | null = this.folder?.name ?? null): Promise<boolean> {
    return new Promise((resolve) => {
      this.answer = resolve
      this.confirm = { action, name }
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

  /** Puts a project in its place; `attaching` is set meanwhile, for the callers that must not see the gap. */
  private async attach(
    handle: FolderHandle,
    store: ProjectStore,
    release: () => void,
    wrote: boolean,
    place: { kind: 'folder' | 'browser'; dir: string | null; releaseDir: (() => void) | null; discard?: string | null },
  ) {
    await this.holdingBarrier(this.attachNow(handle, store, release, wrote, place))
  }

  /**
   * Sets `attaching` until `running` ends, however it ends. Inside a move that already holds it (the attach
   * of `saveInto`) the outer one stays: it is cleared when the whole move is over.
   */
  private async holdingBarrier<T>(running: Promise<T>): Promise<T> {
    if (this.attaching) return running
    const marker = running.then(
      () => undefined,
      () => undefined,
    )
    this.attaching = marker
    try {
      return await running
    } finally {
      if (this.attaching === marker) this.attaching = null
    }
  }

  private async attachNow(
    handle: FolderHandle,
    store: ProjectStore,
    release: () => void,
    wrote: boolean,
    place: { kind: 'folder' | 'browser'; dir: string | null; releaseDir: (() => void) | null; discard?: string | null },
  ) {
    // Remembered before the page says "saved", and before the old project is detached (the project
    // stays reachable meanwhile): a reload right after the first save must still find the folder among
    // the recent projects, or nothing offers it back. Listing again waits for the write to commit
    // (the read transaction queues behind the write one), so keep it. The browser's own storage has
    // one writer (the lock on the project) and is nobody else's to edit, and it is not a folder the
    // user would find among their recent ones.
    if (place.kind === 'folder') {
      await this.deps.recent.remember(handle)
      this.recent = await this.deps.recent.list()
    }
    // The draft's directory lock is kept until its directory is gone: another tab cannot open it meanwhile.
    const draftLock = place.discard ? (this.current?.releaseDir ?? null) : null
    if (draftLock && this.current) this.current.releaseDir = null
    await this.detach()
    // A draft that moved into a folder is deleted before "saved" is published, so a reload after "saved"
    // never finds the project twice. A reload during the removal itself still can: the folder is already
    // among the recent projects and the draft is not gone yet (removing first could lose the offer of the
    // folder). The removal comes only now because the draft's store must be closed first. The draft is
    // deleted only once the folder is confirmed among the recent projects: `remember` reports no failure,
    // and without it nothing would offer the folder back. A draft that cannot be deleted is left behind:
    // the folder holds the project, and the draft stays listed.
    try {
      if (place.discard && (await this.isRemembered(handle))) await this.deps.browser.remove(place.discard).catch(() => undefined)
    } finally {
      draftLock?.()
    }
    const current: Attached = {
      handle,
      store,
      release,
      releaseDir: place.releaseDir,
      expected: {},
      stopWatching: () => undefined,
      kind: place.kind,
      dir: place.dir,
    }
    this.current = current
    this.folder = place.kind === 'folder' ? { name: handle.name } : null
    this.inBrowser = place.kind === 'browser'
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
    if (current.kind === 'folder') current.stopWatching = this.deps.watch(handle, () => this.folderMayHaveChanged())
    if (this.dirty) this.schedule()
  }

  /** Whether the recent projects, as last listed, hold this folder. */
  private async isRemembered(handle: FolderHandle): Promise<boolean> {
    for (const entry of this.recent) {
      if (entry.handle === handle || (await entry.handle.isSameEntry?.(handle)?.catch(() => false))) return true
    }
    return false
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
    current.releaseDir?.()
    current.store.close()
  }

  private reset() {
    this.folder = null
    this.inBrowser = false
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
    // Not while an attach has no project for a moment.
    while (this.attaching) await this.attaching
    const current = this.current
    if (!current) return false
    if (current.kind === 'folder') {
      // The prompt is needed only when the permission is gone, and it opens only from a click.
      let permission = await this.deps.permission(current.handle)
      if (permission !== 'granted') {
        if (this.needsGesture()) return false
        permission = await this.deps.askPermission(current.handle)
      }
      if (permission !== 'granted') return false
      // The folder may have been replaced while the permission was asked.
      if (this.current !== current) return false
    }
    if (this.block === 'permission') {
      this.block = null
      // The notice that told an agent to click is answered by the click.
      this.notice = null
    }
    return this.autosave().then(() => !this.dirty)
  }

  /** The page became visible or focused: a permission that came back, or a folder edited meanwhile. */
  async wake() {
    const current = this.current
    if (!current || current.kind === 'browser') return
    if (this.block === 'permission' && (await this.deps.permission(current.handle)) === 'granted') {
      this.block = null
      void this.autosave()
      return
    }
    this.folderMayHaveChanged()
  }

  /** The page is going to the background: write what waits (a project with no home yet goes to the browser's storage). */
  sleep() {
    if (this.current ? this.dirty && this.block !== 'permission' && this.block !== 'external' : this.dirty && this.deps.browser.available) {
      void this.autosave()
    }
  }

  // ------------------------------------------------------------------ the folder is edited from outside

  private folderMayHaveChanged() {
    if (!this.current || this.current.kind === 'browser') return
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
      const { entries, signature } = await current.store.readFolder()
      const outcome = await this.host.openProject(current.handle.name, entries)
      if (outcome.status === 'refused') {
        // The folder cannot be read as a project now (a checkout half done, a deleted project.json):
        // no write goes into it until the user chooses.
        this.block = 'external'
        this.cancelTimers()
        return false
      }
      // What was read, not a later look: a file changed since is then seen by the next check.
      current.expected = signature
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
    return this.enqueue(() => this.resolveConflict('keepMine'))
  }

  /** The conflict banner: take the folder's version and drop the changes made since the last write. */
  loadDisk(): Promise<boolean> {
    return this.enqueue(() => this.resolveConflict('loadFolder'))
  }

  /**
   * One way out of a conflict. Each loses something (the folder's outside edits, or the project's changes
   * since the last write), so the user is always asked, whoever started it, and only while the folder is in
   * conflict: a command sent with no conflict (or one the user settled meanwhile) changes nothing.
   */
  private async resolveConflict(choice: 'keepMine' | 'loadFolder'): Promise<boolean> {
    const current = this.current
    if (!current || this.block !== 'external') {
      this.notice = 'storage.noConflict'
      return false
    }
    if (!(await this.ask(choice, current.handle.name))) return false
    // The question was open for a while: the same folder must still be in the same conflict.
    if (this.current !== current || this.block !== 'external') {
      this.notice = 'storage.noConflict'
      return false
    }
    if (choice === 'loadFolder') return this.reload('storage.reloaded')
    this.block = null
    return this.write('rewrite')
  }

  /**
   * The one banner the page shows (#474): banners never stack. What stops the work comes first (a write
   * that cannot happen, a folder edited from outside): a notice nobody dismissed must never hide it. Then
   * a notice, which answers what the user just did and goes with its Dismiss, the project of last time,
   * and the advice about the browser's storage. The others wait: they show when this one is gone.
   */
  get banner(): 'permission' | 'full' | 'error' | 'external' | 'reopen' | 'warning' | 'notice' | null {
    if (this.block) return this.block
    if (this.notice) return 'notice'
    if (this.reopen) return 'reopen'
    return this.browserWarning ? 'warning' : null
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
   * On a fresh page: the project that was open last time comes back. A project kept in the browser opens
   * at once; so does a folder whose permission is still granted. Otherwise a banner offers the folder,
   * because the browser wants a click before it gives it back.
   */
  async restore() {
    // The core is made now, not at the first click: a click on Open must reach the chooser while it still has its activation.
    void this.host.prepare?.().catch((error: unknown) => console.error(error))
    this.recent = await this.deps.recent.list()
    let draft: BrowserProject | null = null
    if (this.deps.browser.available) {
      const { projects, incomplete } = await this.deps.browser.list()
      this.browserProjects = projects
      await this.sweep(incomplete)
      draft = await this.firstFree(projects)
    }
    const last = this.recent[0]
    if (this.current || this.worthKeeping()) return
    // The project the musician worked on last wins: a folder is dated by when it was opened or saved
    // into, a project in the browser by its newest file.
    if (draft && (!last || draft.modified > last.at)) {
      // A project kept in the browser needs no permission: it opens by itself, in every browser (#474).
      await this.openBrowserProject(draft.dir)
      return
    }
    if (!last) return
    if ((await this.deps.permission(last.handle)) === 'granted') {
      await this.enqueue(() => this.openHandle(last.handle, { forgetIfGone: last.id }))
    } else {
      this.offer({ id: last.id, name: last.name, kind: 'folder' })
    }
  }

  /** Offers the project of last time with a banner. */
  private offer(reopen: { id: string; name: string; kind: 'folder' | 'browser' }) {
    this.reopen = reopen
  }

  /** The newest of the projects that no other tab has open. */
  private async firstFree(projects: BrowserProject[]): Promise<BrowserProject | null> {
    for (const project of projects) {
      const release = await this.deps.lock(dirLock(project.dir))
      if (release) {
        release()
        return project
      }
    }
    return null
  }

  /**
   * Deletes the directories that hold no project and that no tab is making: a tab closed halfway
   * through a first write or an import left them (and, for an import, perhaps a lot of room). A
   * directory under way in another tab is locked by it, so it is never mistaken for one left behind.
   */
  private async sweep(incomplete: string[]) {
    for (const dir of incomplete) {
      const release = await this.deps.lock(dirLock(dir))
      if (!release) continue
      try {
        await this.deps.browser.remove(dir)
      } catch {
        // Still there; the next start tries again.
      } finally {
        release()
      }
    }
  }

  // ------------------------------------------------------------------ the browser's projects

  /** Deletes a project kept in the browser's storage, after asking; never the one that is open. */
  deleteBrowserProject(dir: string): Promise<boolean> {
    return this.enqueue(async () => {
      if (this.current?.dir === dir) {
        this.notice = 'storage.deleteOpen'
        return false
      }
      // Only a project the browser lists: a name that is not in its list is never handed to the storage.
      await this.refreshBrowserProjects()
      if (!this.browserProjects.some((project) => project.dir === dir)) {
        if (this.reopen?.kind === 'browser' && this.reopen.id === dir) this.reopen = null
        this.notice = 'storage.browserGone'
        return false
      }
      // A project that another tab has open is not ours to delete.
      const release = await this.deps.lock(dirLock(dir))
      if (!release) {
        this.notice = 'storage.locked'
        return false
      }
      try {
        const name = (this.browserProjects.find((p) => p.dir === dir) ?? this.reopen)?.name ?? null
        if (!(await this.ask('delete', name))) return false
        await this.deps.browser.remove(dir)
        if (this.reopen?.kind === 'browser' && this.reopen.id === dir) this.reopen = null
        return true
      } catch (error) {
        this.errorDetail = error instanceof Error ? error.message : String(error)
        this.notice = 'storage.deleteFailed'
        return false
      } finally {
        release()
        await this.refreshBrowserProjects()
      }
    })
  }

  // ------------------------------------------------------------------ .faulkpack

  /** Export: hands the project to the user as a `.faulkpack`, as it is now (saved or not). */
  exportPack(): Promise<boolean> {
    return this.enqueue(() => this.doExport())
  }

  private async doExport(): Promise<boolean> {
    const current = this.current
    let frame: SaveFrame | null
    try {
      frame = this.host.save('everything')
    } catch (error) {
      console.error(error)
      this.notice = 'pack.exportFailed'
      return false
    }
    if (!frame) return false
    // Only to read the project as it is; the home is not told it holds anything new.
    this.host.saved(false)
    if (frame.status === 'failed') {
      this.errorDetail = frame.message
      this.notice = 'pack.exportFailed'
      return false
    }
    try {
      // The media a save does not rewrite come from the home; the frame is what the project is now.
      const files: Record<string, Uint8Array> = {}
      if (current) for (const blob of await current.store.readBlobs()) files[blob.path] = blob.bytes
      for (const entry of frame.entries) if (entry.type === 'write') files[entry.path] = entry.bytes
      const packed = await this.host.packExport(Object.entries(files).map(([path, bytes]) => ({ path, bytes })))
      if (packed.status === 'failed') {
        this.errorDetail = packed.message
        this.notice = 'pack.exportFailed'
        return false
      }
      // A file name: what the systems refuse in one (and control characters) becomes an underscore.
      const name =
        [...(this.host.projectName() ?? '')]
          .map((ch) => (ch.charCodeAt(0) < 32 || '\\/:*?"<>|'.includes(ch) ? '_' : ch))
          .join('')
          .trim() || 'project'
      this.deps.download(`${name}.faulkpack`, new Blob([packed.zip as Uint8Array<ArrayBuffer>], { type: 'application/zip' }))
      this.notice = packed.missingBlobs > 0 ? 'pack.missingMedia' : null
      return true
    } catch (error) {
      this.errorDetail = error instanceof Error ? error.message : String(error)
      this.notice = 'pack.exportFailed'
      return false
    }
  }

  /** Import: the user picks a `.faulkpack`; its project is kept in the browser's storage and opened. Call straight from a click. */
  async importPack(): Promise<boolean> {
    if (this.needsGesture()) return false
    const file = await this.deps.pickPack()
    if (!file) return false
    return this.enqueue(() => this.importFile(file))
  }

  private async importFile(file: File): Promise<boolean> {
    if (file.size > MAX_PACK_BYTES) {
      this.notice = 'pack.tooBig'
      return false
    }
    // Read and checked before anything is touched: a file that is no pack changes nothing.
    let unpacked: PackImport
    try {
      unpacked = await this.host.packImport(new Uint8Array(await file.arrayBuffer()))
    } catch (error) {
      this.errorDetail = error instanceof Error ? error.message : String(error)
      this.notice = 'pack.invalid'
      return false
    }
    if (unpacked.status === 'failed') {
      this.errorDetail = unpacked.message
      this.notice = 'pack.invalid'
      return false
    }
    const files = unpacked.files
    const id = projectIdOf(files)
    const name = file.name.replace(/\.faulkpack$/i, '') || 'pack'
    // The pack keeps its identity, so what it gives back is the project it was made from. A project
    // of that identity that is already here (open, or kept in the browser) is never overwritten by it.
    if (id) {
      const kept = this.deps.browser.available ? (await this.deps.browser.list()).projects.find((p) => p.id === id) : undefined
      if (kept || this.host.projectId() === id) {
        // Names the project and, when it is kept in the browser and not the one open, offers to open it.
        const open = this.host.projectId() === id ? null : (kept?.dir ?? null)
        this.notice = open === null ? 'pack.alreadyOpen' : 'pack.alreadyHere'
        this.noticeVars = { name: kept?.name ?? name }
        this.noticeOpen = open
        return false
      }
    }
    if (!this.deps.browser.available) return this.openImported(name, files)
    // Into a directory of the browser's storage first, as a pack is written into an empty folder: the
    // page's own project is untouched until the pack is whole there, `project.json` last.
    const dir = crypto.randomUUID()
    const releaseDir = await this.deps.lock(dirLock(dir))
    let handle: FolderHandle
    try {
      handle = await this.deps.browser.create(dir)
      const store = await this.deps.openStore(handle)
      try {
        const rank = (path: string) => (path.startsWith('blobs/') ? 0 : path === 'project.json' ? 2 : 1)
        const ordered = [...files].sort((a, b) => rank(a.path) - rank(b.path))
        await store.apply(ordered.map((f): SaveEntry => ({ type: 'write', path: f.path, bytes: f.bytes })))
      } finally {
        store.close()
      }
    } catch (error) {
      await this.deps.browser.remove(dir).catch(() => undefined)
      releaseDir?.()
      this.errorDetail = error instanceof Error ? error.message : String(error)
      this.notice = error instanceof StoreError && error.code === 'full' ? 'storage.browserFull' : 'pack.writeFailed'
      return false
    }
    releaseDir?.()
    const opened = await this.openHandle(handle, { kind: 'browser', dir })
    // Declined (the question before replacing a project) or refused: the copy made for it goes.
    if (!opened) await this.deps.browser.remove(dir).catch(() => undefined)
    else this.persistOnce()
    await this.refreshBrowserProjects()
    return opened
  }

  /** Where the browser keeps nothing: the pack's project opens in the tab alone. */
  private async openImported(name: string, files: { path: string; bytes: Uint8Array }[]): Promise<boolean> {
    if (!(await this.makeRoom('open'))) return false
    const entries: FolderEntry[] = files.map((f) =>
      f.path.startsWith('blobs/') && !needsBytes(f.path, f.bytes.subarray(0, 8)) ? { path: f.path, size: f.bytes.length } : { path: f.path, bytes: f.bytes },
    )
    const outcome = await this.host.openProject(name, entries)
    if (outcome.status === 'refused') return false
    await this.detach()
    this.reset()
    return true
  }
}
