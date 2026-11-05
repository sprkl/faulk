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
import { ImportError } from '../audio/import/abi'
import type { CacheReport, ImportedMedia, MediaRef } from '../audio/import/pipeline'
import { peaksPath } from '../audio/peaks/format'
import type { MessageKey } from '../i18n'
import { needsBytes, type FolderEntry, type SaveEntry, type SaveFrame, type SaveMode } from '../project/core'
import type { PackExport, PackImport, PackIO } from '../project/core'
import type { OpenOutcome } from '../stores/open'
import { cleanName } from '../export/audio-export.svelte'
import { checkOpenUrl, downloadPack, packNameOf, streamPack, type Streamed } from './open-url'
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
  /** `project.rename` through the command path (Save as names the project after the folder it creates); false when the project did not take the name. */
  renameProject(name: string): boolean
  /** Writes the `.faulkpack` of a project folder to `io.sink`, in pieces (see `ProjectCore.packExport`). */
  packExport(entries: FolderEntry[], io: PackIO): Promise<PackExport>
  /** The files of a `.faulkpack`, validated (see `ProjectCore.packImport`). */
  packImport(zip: Uint8Array): Promise<PackImport>
  /** Makes the project core ahead of a click that will need it, so the click still has its permission to prompt. */
  prepare?(): Promise<void>
  /** The audio files the project's clips play (AU-1), whose PCM caches an opening makes again. */
  audioMedia?(): MediaRef[]
  /** The project's place changed: its folder (or browser directory), where the engine reads audio files (AU-3); `null`: none. */
  folderChanged?(handle: FileSystemDirectoryHandle | null): void
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
  /**
   * Save as asks the project's name first (an in-app field, prefilled with `initial`): the project then goes into a
   * folder of that name inside the folder the user picks. `null` when the user cancelled. Without it (tests) the picked
   * folder itself is the project's folder.
   */
  askName?(initial: string): Promise<string | null>
  /** The browser's own storage (the OPFS), where drafts and browser projects live. */
  browser: BrowserProjects
  /** Asks the user for a `.faulkpack` (a file chooser; call straight from a click); `null` when cancelled. */
  pickPack(): Promise<File | null>
  /** Hands a file to the user (a download). */
  download(name: string, blob: Blob): void
  /**
   * Asks the user where a big file goes and opens it to be written in pieces (call straight from a click): the
   * file, `'cancelled'` when the user closed the dialog, `null` when the browser has no such dialog or no click
   * is behind the call (the file is then a download, kept by the browser). Absent in tests.
   */
  saveFile?(name: string): Promise<PackTarget | 'cancelled' | null>
  /** The bytes the browser can still keep for the page, `null` when it does not say. Absent in tests. */
  freeSpace?(): Promise<number | null>
  /** The browser's `fetch`, for a pack opened by its link (`project.open_url`). */
  fetch: typeof fetch
  /** Where the page itself is served from (`location.hostname`): plain http links are for a page on this computer only. */
  pageHostname(): string
}

/** A file written in pieces (a `.faulkpack` the user chose the place of): `close` completes it, `abort` leaves nothing. */
export interface PackTarget {
  write(chunk: Uint8Array): Promise<void>
  close(): Promise<void>
  abort(): Promise<void>
}

/** A pack with this many bytes of media or more goes to a file the user chooses; a smaller one is a download. */
export const PACK_TO_FILE_BYTES = 64 << 20

/**
 * The biggest `.faulkpack` the page reads whole, where the browser keeps no projects (the core holds the pack and its files
 * in memory, `crates/project-wasm` `pack.rs`). Where it keeps them, a pack is read in pieces, whatever its size (#682).
 */
export const MAX_PACK_BYTES = 192 << 20

/** The biggest `.faulkpack` the page downloads into the browser's storage: the format's own limit (16 GiB of files), and its directory. */
export const MAX_STREAMED_PACK_BYTES = 16 * 2 ** 30 + (64 << 20)

/** Where a pack opened by its link is downloaded, in the new project's directory, until it is read (`.faulk/tmp/`, swept). */
export const DOWNLOAD_PATH = '.faulk/tmp/download.faulkpack'

/** The pieces a download is handed to the store's Worker in. */
const DOWNLOAD_PIECE_BYTES = 1 << 20

/** An import under way: its cancel, and whether its project started to open (then a cancel comes too late). */
interface ImportRun {
  abort: AbortController
  committed: boolean
}

/** What `cancelImport` did: the imports it stopped, and those it left to finish because their project had started to open. */
export interface ImportCancel {
  cancelled: number
  tooLate: number
}

/**
 * A file read whole, piece by piece (`File.stream()`; else at once, the cancel then only drops what was read), so that a
 * cancel stops the read at the next piece. `null` when cancelled. `onProgress`: the fraction read.
 */
export async function readWhole(file: Blob, signal: AbortSignal, onProgress: (fraction: number) => void): Promise<Uint8Array | null> {
  if (signal.aborted) return null
  if (typeof file.stream !== 'function') {
    const bytes = new Uint8Array(await file.arrayBuffer())
    return signal.aborted ? null : bytes
  }
  const out = new Uint8Array(file.size)
  const reader = file.stream().getReader()
  let at = 0
  try {
    for (;;) {
      if (signal.aborted) {
        await reader.cancel().catch(() => undefined)
        return null
      }
      const { done, value } = await reader.read()
      if (done) break
      // A file that grew under the reader: the pack is no longer the one that was picked.
      if (at + value.length > out.length) throw new Error('the file changed while it was read')
      out.set(value, at)
      at += value.length
      onProgress(out.length === 0 ? 1 : at / out.length)
    }
  } finally {
    reader.releaseLock()
  }
  if (signal.aborted) return null
  return at === out.length ? out : out.subarray(0, at)
}

/** A new directory of the browser's storage made for an import, its lock and its store. */
interface ImportDir {
  dir: string
  handle: FolderHandle
  store: ProjectStore
  release: (() => void) | null
}

/** Where `writeInto` puts a project: the browser's storage, or a folder (`project` and `folder`: Save as named the project). */
type SaveTarget = { kind: 'folder'; handle: FolderHandle; project?: string; folder?: string } | { kind: 'browser' }

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
  /**
   * The project of last time that would not open (locked by another tab, or unreadable) while its notice is
   * shown (#504): the notice then offers to try again and a way to another project. Cleared with every new notice.
   */
  failedReopen = $state.raw<{ id: string; name: string; kind: 'folder' | 'browser' } | null>(null)
  get notice(): MessageKey | null {
    return this.shownNotice
  }
  set notice(key: MessageKey | null) {
    this.shownNotice = key
    this.failedReopen = null
    this.noticeVars = null
    this.noticeOpen = null
    if (key !== null) for (const watcher of this.noticeWatchers) watcher(key)
  }
  /** Not state: nothing shows them. */
  private noticeWatchers: ((key: MessageKey) => void)[] = []
  /**
   * Calls `watcher` with each notice raised until the returned function is called: what an agent is told when a
   * storage action it started was not done (AG-4). A hint only: a notice of another task in the meantime is counted too.
   */
  watchNotices(watcher: (key: MessageKey) => void): () => void {
    this.noticeWatchers = [...this.noticeWatchers, watcher]
    return () => {
      this.noticeWatchers = this.noticeWatchers.filter((w) => w !== watcher)
    }
  }
  /** A question the interface is asking (New or Open on a project whose changes cannot be kept). */
  confirm = $state.raw<{ action: Discard; name: string | null } | null>(null)
  /** The host a pack is being downloaded from (`project.open_url`), while it is. */
  downloading = $state<string | null>(null)
  /** The share of the download received (0 to 1) when the host announced its length, else `null`. */
  downloadProgress = $state<number | null>(null)
  /** The host a pack could not be downloaded from: the banner offers to import a file instead. */
  downloadFailed = $state<string | null>(null)
  /** The share of the media written so far (0 to 1) while a `.faulkpack` is being made, else `null`. */
  exporting = $state<number | null>(null)
  /** The share of the pack read so far (0 to 1) while a `.faulkpack` is being imported into the browser's storage, else `null`. */
  importing = $state<number | null>(null)
  /** A link of the address (`/app?open=`) waiting for the user's answer before anything is downloaded. */
  linkOffer = $state.raw<{ url: string; host: string } | null>(null)

  /**
   * What the last write cost: `coreMs` is time on the page's main thread (the core computing the
   * write), `writeMs` the time until the store, in its Worker, had applied it. For measurements.
   */
  lastWrite: { coreMs: number; writeMs: number; files: number; bytes: number } | null = null

  private current: Attached | null = null
  /** Stops the export under way, if any. */
  private exportAbort: AbortController | null = null
  /** An export is under way, from the click on: the file dialog and the preparation included. */
  private exportBusy = false
  /** The imports asked for and not over (#682): `cancelImport` stops them all, but those whose project started to open. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- never read by the interface
  private readonly importRuns = new Map<AbortSignal, ImportRun>()
  private queue: Promise<unknown> = Promise.resolve()
  /** The downloads of `openUrl`, one at a time, outside `queue` so a slow host never holds a save back. */
  private downloads: Promise<unknown> = Promise.resolve()
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
   * Save as: the user names the project, picks a folder, and the project is written whole into a folder of that
   * name inside it (an empty one, or one made now), with a new identity when it already had a folder. The folder
   * dialog opens from the click that confirmed the name. Call straight from a click.
   */
  async saveAs(): Promise<boolean> {
    if (!this.supported) {
      this.notice = 'storage.unsupported'
      return false
    }
    // The chooser opens only from a click: an agent without one is told so, and nothing is chosen.
    if (this.needsGesture()) return false
    let name: string | null = null
    if (this.deps.askName) {
      name = await this.deps.askName(this.host.projectName() ?? '')
      name = name?.trim() ?? null
      // Cancelled, or nothing typed (the field refuses it too): nothing was chosen.
      if (!name) return false
      // The confirming click is the gesture of the folder dialog, a moment ago at most.
      if (this.needsGesture()) return false
    }
    const picked = await this.deps.picker.pick()
    if (picked.status === 'cancelled') return false
    if (picked.status === 'refused') {
      this.notice = 'storage.refused'
      return false
    }
    this.notice = null
    if (name === null) return this.enqueue(() => this.saveInto({ kind: 'folder', handle: picked.handle }))
    const folder = cleanName(name)
    const made = await this.makeFolder(picked.handle, folder)
    if (!made) return false
    const project = name
    const saved = await this.enqueue(() => this.saveInto({ kind: 'folder', handle: made.handle, project, folder }))
    // A folder made for this save and left empty goes away (`removeEntry` refuses a folder that holds anything).
    if (!saved && made.created) await picked.handle.removeEntry(folder).catch(() => undefined)
    return saved
  }

  /** The folder `name` inside `parent`, made when it does not exist yet; `null` (with the notice) when the browser refuses. */
  private async makeFolder(parent: FolderHandle, name: string): Promise<{ handle: FolderHandle; created: boolean } | null> {
    try {
      return { handle: await parent.getDirectoryHandle(name), created: false }
    } catch (error) {
      if (!(error instanceof DOMException && error.name === 'NotFoundError')) return this.folderFailed(error)
    }
    try {
      return { handle: await parent.getDirectoryHandle(name, { create: true }), created: true }
    } catch (error) {
      return this.folderFailed(error)
    }
  }

  private folderFailed(error: unknown): null {
    const refused = error instanceof DOMException && (error.name === 'NotAllowedError' || error.name === 'SecurityError')
    // A file called like that (TypeMismatchError) or anything else the browser throws.
    this.notice = refused ? 'storage.refused' : 'storage.saveFailed'
    return null
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
  private saveInto(target: SaveTarget): Promise<boolean> {
    // The whole move is one gap for Save and the export folder: the blobs are copied and the frame written
    // while the old home is still current, and a Save then would ask for a second folder.
    return this.holdingBarrier(this.writeInto(target))
  }

  private async writeInto(target: SaveTarget): Promise<boolean> {
    this.cancelTimers()
    const from = this.current
    const promotedFrom = from?.kind === 'browser' && target.kind === 'folder' ? from : null
    let store: ProjectStore | null = null
    let release: (() => void) | null = null
    let releaseDir: (() => void) | null = null
    let dir: string | null = null
    let frame: SaveFrame | null = null
    let handle: FolderHandle
    // The name the project had before Save as renamed it: given back when the write does not happen.
    let renamedFrom: string | null = null
    const undoRename = () => {
      if (renamedFrom !== null) this.host.renameProject(renamedFrom)
      renamedFrom = null
    }
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
        const named = target.kind === 'folder' ? target.folder : undefined
        this.notice = named ? 'storage.nameTaken' : 'storage.notEmpty'
        if (named) this.noticeVars = { name: named }
        return await this.abandon(null, store, null, releaseDir, dir)
      }
      // Save as names the project: through the command path, once the folder is known to take it, and just before
      // the frame is computed so the folder holds the new name.
      const before = this.host.projectName()
      if (target.kind === 'folder' && target.project !== undefined && target.project !== before) {
        if (!this.host.renameProject(target.project)) throw new Error('the project did not take its new name')
        renamedFrom = before ?? ''
      }
      const startedComputing = performance.now()
      frame = this.host.save(from?.kind === 'folder' ? 'copy' : 'everything')
      const coreMs = performance.now() - startedComputing
      if (!frame) {
        undoRename()
        return await this.abandon(null, store, null, releaseDir, dir)
      }
      if (frame.status === 'failed') throw new Error(frame.message)
      if (promotedFrom) {
        // Same project, same identity: its lock moves to the folder instead of being taken twice.
        release = promotedFrom.release
      } else {
        const id = this.host.projectId()
        release = id ? await this.deps.lock(id) : () => undefined
        if (!release) {
          this.notice = 'storage.locked'
          undoRename()
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
      undoRename()
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

  /**
   * Open a recent project. Call straight from a click: the browser may ask for the permission again. `allowed`: the
   * user already allowed an agent to replace the project (AG-4), so the question about unsaved changes is not asked again.
   */
  async openRecent(id: string, allowed = false): Promise<boolean> {
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
    return this.enqueue(() => this.openHandle(entry.handle, { forgetIfGone: id, allowed }))
  }

  /** A reopen under way: a second click on Retry meanwhile joins it instead of finding nothing to reopen. */
  private reopening: Promise<boolean> | null = null

  /**
   * The banner after a reload: the project that was open comes back with the user's click. `allowed`: as for
   * `openRecent`; a call that joins a reopen under way takes that reopen's answer.
   */
  reopenLast(allowed = false): Promise<boolean> {
    this.reopening ??= this.runReopenLast(allowed).finally(() => (this.reopening = null))
    return this.reopening
  }

  private async runReopenLast(allowed: boolean): Promise<boolean> {
    // The project that would not open comes second: its notice offers to try it again (#504).
    const last = this.reopen ?? this.failedReopen
    if (!last) {
      this.notice = 'storage.nothingToReopen'
      return false
    }
    // A retry of a project that already failed keeps its actions through a permission notice too; a first try (the reopen
    // banner) does not: refusing the browser's prompt there is the user's own answer.
    const retrying = this.failedReopen !== null
    const before = { notice: this.shownNotice, vars: this.noticeVars, open: this.noticeOpen, failed: this.failedReopen }
    // A notice from before is not this attempt's: only one that this attempt raises may carry the actions.
    this.notice = null
    let opened: boolean
    try {
      opened = await (last.kind === 'browser' ? this.openBrowserProject(last.id, allowed) : this.openRecent(last.id, allowed))
    } catch (error) {
      // The attempt itself broke (a stale handle): the notice and its actions stay, and the error goes on.
      this.restoreNotice(before)
      throw error
    }
    if (!opened) {
      this.markReopenFailed(last, retrying)
      // The attempt ended with no notice of its own (the question about unsaved changes was declined): the notice and its actions stay.
      if (this.shownNotice === null && before.failed) this.restoreNotice(before)
    }
    return opened
  }

  private restoreNotice(before: {
    notice: MessageKey | null
    vars: { name: string } | null
    open: string | null
    failed: { id: string; name: string; kind: 'folder' | 'browser' } | null
  }) {
    this.notice = before.notice
    this.noticeVars = before.vars
    this.noticeOpen = before.open
    this.failedReopen = before.failed
  }

  /**
   * Keeps the project that did not open for the notice's actions, when the notice is one a second try can answer:
   * the project is locked or unreadable, or, for a retry, the permission was denied or a click was missing.
   */
  private markReopenFailed(target: { id: string; name: string; kind: 'folder' | 'browser' }, retrying = false) {
    const notice = this.shownNotice
    const permission = notice === 'storage.permissionDenied' || notice === 'storage.needsClick'
    if (notice === 'storage.locked' || notice === 'project.open.unreadable' || (retrying && permission)) this.failedReopen = target
  }

  /**
   * Open a project kept in the browser's storage (a draft, or a project of a browser with no folder). Only one the
   * browser lists: a name that is not in its list is never handed to the storage. `allowed`: as for `openRecent`.
   */
  openBrowserProject(dir: string, allowed = false): Promise<boolean> {
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
      const opened = await this.openHandle(handle, { kind: 'browser', dir, allowed })
      if (opened) this.persistOnce()
      return opened
    })
  }

  private async openHandle(
    handle: FolderHandle,
    options: { forgetIfGone?: string; kind?: 'folder' | 'browser'; dir?: string; allowed?: boolean } = {},
  ): Promise<boolean> {
    const kind = options.kind ?? 'folder'
    const dir = options.dir ?? null
    // The project that is open already: nothing to open, and its own lock must not refuse it.
    if (this.current && (kind === 'browser' ? this.current.dir === dir : await this.current.handle.isSameEntry?.(handle)?.catch(() => false))) return true
    if (!(await this.makeRoom('open', options.allowed))) return false
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

  /**
   * New project, named `name` (else the page's draft name): asks first when changes would be lost, unless the user
   * already `allowed` an agent to replace the project (AG-4: one question, never two).
   */
  newProject(name?: string, allowed = false): Promise<boolean> {
    return this.enqueue(async () => {
      if (!(await this.makeRoom('new', allowed))) return false
      if (!(await this.host.newProject(name))) return false
      await this.detach()
      this.reset()
      return true
    })
  }

  /**
   * The project is about to be replaced. With a folder, the changes are written first and the
   * user is asked only if they could not be. Without one, the user is asked whenever the
   * project was changed. True when it may go ahead. `allowed`: the user already said yes to an agent
   * doing it (its question warned that changes that cannot be kept are lost), so nothing is asked.
   */
  private async makeRoom(action: Discard, allowed = false): Promise<boolean> {
    if (this.current && (this.host.dirty() ?? false)) await this.write('changed')
    // A change still waiting for its first write into the browser's storage is kept there now, not discarded.
    else if (!this.current && this.worthKeeping() && this.deps.browser.available) await this.startBrowserProject()
    this.dirty = this.worthKeeping()
    if (!this.dirty) return true
    return this.ask(action, undefined, allowed)
  }

  /** Asks the user (`confirm`); `allowed` answers yes at once: the user already allowed the agent that asks (AG-4). */
  private ask(action: Discard, name: string | null = this.folder?.name ?? null, allowed = false): Promise<boolean> {
    if (allowed) return Promise.resolve(true)
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
    // The audio cache work of this attach is awaitable from its first instant, not only once it starts at
    // the end of the attach: a hook that asks before then waits for it instead of reading the previous
    // (resolved) promise and an empty report (#707). Settled with the work, or at once when the attach
    // ends without it.
    let settle: (done?: Promise<void>) => void = () => {}
    this.audioCacheDone = new Promise<void>((resolve) => (settle = resolve))
    try {
      await this.holdingBarrier(this.attachNow(handle, store, release, wrote, place, settle))
    } finally {
      settle()
    }
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
    settleAudioCache: (done: Promise<void>) => void,
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
    this.host.folderChanged?.(handle)
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
    settleAudioCache(this.refreshAudioCache(current))
  }

  // ------------------------------------------------------------------ audio files (AU-1)

  /** The PCM caches made again after the last attach, in the background (`ensureAudioCache`); `null` meanwhile. */
  audioCache = $state.raw<CacheReport | null>(null)
  /** That background work, for tests and the dev hook to wait on. */
  audioCacheDone: Promise<void> = Promise.resolve()

  /**
   * The caches a folder lacks are made again where its disk is held: here, in both modes (the helper
   * holds no disk). In the background, never in the storage's queue (it writes only `.faulk/cache/`,
   * which no save, scan or copy reads).
   */
  private refreshAudioCache(current: Attached): Promise<void> {
    const media = this.host.audioMedia?.() ?? []
    this.audioCache = null
    // Stopped when the project leaves this place (`detach`): it would make caches for nothing. The
    // waveform peaks asked for later in this attach (`makePeaks`) stop with it.
    const abort = new AbortController()
    this.audioCacheAbort = abort
    this.peaksEpoch++
    if (media.length === 0) return Promise.resolve()
    return current.store
      .ensureAudioCache(media, undefined, abort.signal)
      .then((report) => {
        if (this.current === current) this.audioCache = report
      })
      .catch((error: unknown) => {
        // The project stays usable: a clip whose cache is missing waits for the next opening.
        if (!(error instanceof StoreError || error instanceof ImportError)) console.error(error)
      })
  }

  private audioCacheAbort: AbortController | null = null

  // ------------------------------------------------------------------ waveform peaks (AU-2)

  /** Counts the attaches: a waveform whose peaks this place lacked is tried again in the next one. */
  peaksEpoch = $state(0)

  /**
   * The peaks file of `hash` in the project's place (`.faulk/cache/peaks/`, read by the store's
   * Worker), or `null` when there is none, or no place.
   */
  async readPeaks(hash: string): Promise<Uint8Array | null> {
    const current = this.current
    if (!current) return null
    try {
      return await current.store.read(peaksPath(hash))
    } catch {
      return null
    }
  }

  /** The media whose peaks a view asked for, waiting for the one run at a time (`makePeaks`). */
  private peaksQueue: Promise<void> = Promise.resolve()

  /**
   * Has the peaks of `media` made in the project's place, in the background: one media at a time
   * (each is a whole decode of its blob in the Worker; `PeaksCache` asks only once the opening's own
   * work is over), and stopped
   * when the project leaves this place. Never rejects: a media it cannot make stays without peaks.
   */
  makePeaks(media: MediaRef): Promise<void> {
    const current = this.current
    const signal = this.audioCacheAbort?.signal
    if (!current || !signal || signal.aborted) return Promise.resolve()
    const run = this.peaksQueue.then(async () => {
      if (this.current !== current || signal.aborted) return
      await current.store.ensureAudioCache([media], undefined, signal)
    })
    this.peaksQueue = run.catch(() => undefined)
    return this.peaksQueue
  }

  /**
   * Imports an audio file into the project's home (AU-1): its bytes into `blobs/`, its decoded audio
   * checked and cached. A project with no home yet gets its draft in the browser's storage first,
   * where there is one (the file needs a place to live before a clip refers to it). In the storage's
   * queue: a save, a save as or an opening never runs in the middle of an import, so a save as never
   * copies the blobs without the one being imported. The blob's stamp joins the stamps of the page's
   * own writes, so the folder's watcher never takes the import for an outside change.
   */
  importAudio(file: File, onProgress?: (fraction: number) => void, signal?: AbortSignal): Promise<ImportedMedia> {
    this.cancelTimers()
    return this.enqueue(async () => {
      while (this.attaching) await this.attaching
      // Cancelled while it waited in the queue: nothing started, nothing to keep.
      if (signal?.aborted) throw new ImportError('cancelled', 'cancelled before it started')
      if (!this.current && this.deps.browser.available) await this.startBrowserProject(true)
      const current = this.current
      if (!current) throw new ImportError('unavailable', 'the project has no place to keep audio files: save it first')
      if (this.block === 'permission') throw new StoreError('permission', 'the folder needs the permission again')
      try {
        const outcome = await current.store.importAudio(file, onProgress, signal)
        if (this.current === current) current.expected[outcome.blob.path] = outcome.blob.stamp
        // Its peaks are on disk now: a waveform of that file left without them is tried again.
        if (this.current === current && outcome.peaks) this.peaksEpoch++
        return outcome.media
      } finally {
        // What waited behind the import (an edit made meanwhile) is written now.
        if (this.current === current && (this.host.dirty() ?? false)) this.schedule()
      }
    })
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
    this.audioCacheAbort?.abort()
    this.audioCacheAbort = null
    if (this.externalTimer) clearTimeout(this.externalTimer)
    this.externalTimer = null
    this.current = null
    if (!current) return
    this.host.folderChanged?.(null)
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
      // The folder reads again: a waveform left without its peaks is tried again.
      this.peaksEpoch++
    }
    return this.autosave().then(() => !this.dirty)
  }

  /** The page became visible or focused: a permission that came back, or a folder edited meanwhile. */
  async wake() {
    const current = this.current
    if (!current || current.kind === 'browser') return
    if (this.block === 'permission' && (await this.deps.permission(current.handle)) === 'granted') {
      this.block = null
      this.peaksEpoch++
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

  /**
   * The conflict banner: keep the project as it is; the next write replaces the folder's files. `allowed`: the user
   * already allowed the agent that asks (AG-4).
   */
  keepMine(allowed = false): Promise<boolean> {
    return this.enqueue(() => this.resolveConflict('keepMine', allowed))
  }

  /** The conflict banner: take the folder's version and drop the changes made since the last write. `allowed`: as for `keepMine`. */
  loadDisk(allowed = false): Promise<boolean> {
    return this.enqueue(() => this.resolveConflict('loadFolder', allowed))
  }

  /** Whether the folder is in conflict now (`external`): only then does a way out of it do anything. */
  get inConflict(): boolean {
    return this.current !== null && this.block === 'external'
  }

  /**
   * One way out of a conflict. Each loses something (the folder's outside edits, or the project's changes
   * since the last write), so the user is always asked, whoever started it (an agent's question, AG-4, or this
   * one), and only while the folder is in conflict: a command sent with no conflict (or one the user settled
   * meanwhile) changes nothing.
   */
  private async resolveConflict(choice: 'keepMine' | 'loadFolder', allowed: boolean): Promise<boolean> {
    const current = this.current
    if (!current || this.block !== 'external') {
      this.notice = 'storage.noConflict'
      return false
    }
    if (!(await this.ask(choice, current.handle.name, allowed))) return false
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
  get banner():
    'permission' | 'full' | 'error' | 'external' | 'reopen' | 'warning' | 'notice' | 'exporting' | 'importing' | 'downloading' | 'downloadFailed' | null {
    if (this.block) return this.block
    // A pack being written (#617) or read (#682), then a pack opened by its link (#497): the download under way, then why it failed.
    if (this.exporting !== null) return 'exporting'
    if (this.importing !== null) return 'importing'
    if (this.downloading) return 'downloading'
    if (this.downloadFailed) return 'downloadFailed'
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

  /**
   * A link from the address (`?open=`): it comes from anywhere, so nothing is downloaded before the user says
   * yes. A link the page would not fetch is refused at once with a notice, and no question is asked.
   */
  offerLink(url: string) {
    const checked = checkOpenUrl(url, this.deps.pageHostname())
    if (checked.status !== 'ok') {
      this.notice = 'openUrl.refused'
      return
    }
    this.linkOffer = { url, host: checked.host }
  }

  /** The user's answer to `linkOffer`: the link to open (the caller runs `project.open_url`), or `null`. */
  answerLink(open: boolean): string | null {
    const offer = this.linkOffer
    this.linkOffer = null
    return open && offer ? offer.url : null
  }

  /** Says the banner for a failed download was read. */
  dismissDownloadFailed() {
    this.downloadFailed = null
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
      const opened = await this.openBrowserProject(draft.dir)
      if (!opened) this.markReopenFailed({ id: draft.dir, name: draft.name, kind: 'browser' })
      return
    }
    if (!last) return
    if ((await this.deps.permission(last.handle)) === 'granted') {
      const opened = await this.enqueue(() => this.openHandle(last.handle, { forgetIfGone: last.id }))
      if (!opened) this.markReopenFailed({ id: last.id, name: last.name, kind: 'folder' })
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

  /**
   * Deletes a project kept in the browser's storage, after asking; never the one that is open. `allowed`: the user
   * already allowed the agent that asks (AG-4), so the question is not asked twice.
   */
  deleteBrowserProject(dir: string, allowed = false): Promise<boolean> {
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
        if (!(await this.ask('delete', name, allowed))) return false
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

  /**
   * Export: hands the project to the user as a `.faulkpack`, as it is now (saved or not). The pack is written
   * in pieces, the media read from the folder in pieces, so a project of any size takes the same memory
   * (#617). With much media it goes to a file the user chooses (call straight from a click); otherwise, or
   * where the browser has no such dialog, to a download.
   */
  async exportPack(): Promise<boolean> {
    // One at a time, from the click to the end: the file dialog is part of it.
    if (this.exportBusy) return false
    this.exportBusy = true
    // A cancel counts from the click on: during the dialog and the preparation too.
    const abort = new AbortController()
    this.exportAbort = abort
    try {
      // The place of the pack first, before anything waits (the queue, a store): the browser opens its file
      // dialog only for a call that follows a click closely.
      const target = await this.packTarget(`${this.packFileName()}.faulkpack`, await this.mediaBytes())
      if (target === 'cancelled') return false
      if (target === 'noRoom') {
        this.notice = 'pack.noRoom'
        return false
      }
      const prepared = await this.enqueue(() => this.prepareExport())
      if (!prepared || abort.signal.aborted) {
        await target.abort().catch(() => undefined)
        if (prepared) this.notice = 'pack.exportCancelled'
        return false
      }
      // Outside the queue: a long export never holds a save back.
      return await this.writePack(prepared, target, abort)
    } finally {
      this.exportBusy = false
      this.exportAbort = null
    }
  }

  /** A file name for the pack: what the systems refuse in one (and control characters) becomes an underscore. */
  private packFileName(): string {
    return (
      [...(this.host.projectName() ?? '')]
        .map((ch) => (ch.charCodeAt(0) < 32 || '\\/:*?"<>|'.includes(ch) ? '_' : ch))
        .join('')
        .trim() || 'project'
    )
  }

  /** The bytes of media the home holds (what an export reads), `0` when it cannot say. */
  private async mediaBytes(): Promise<number> {
    try {
      let bytes = 0
      for (const entry of (await this.current?.store.listBlobs()) ?? []) if ('size' in entry) bytes += entry.size
      return bytes
    } catch {
      return 0
    }
  }

  /** Stops the export under way, whatever it is doing (choosing the file, preparing, writing). */
  cancelExport() {
    this.exportAbort?.abort()
  }

  /** What the export writes: the project as it is now, as a folder. `null` when there is nothing to export. */
  private async prepareExport(): Promise<{ entries: FolderEntry[]; store: ProjectStore | null } | null> {
    const current = this.current
    let frame: SaveFrame | null
    try {
      frame = this.host.save('everything')
    } catch (error) {
      console.error(error)
      this.notice = 'pack.exportFailed'
      return null
    }
    if (!frame) return null
    // Only to read the project as it is; the home is not told it holds anything new.
    this.host.saved(false)
    if (frame.status === 'failed') {
      this.errorDetail = frame.message
      this.notice = 'pack.exportFailed'
      return null
    }
    try {
      // The media a save does not rewrite come from the home; the frame is what the project is now.
      const files: Record<string, FolderEntry> = {}
      if (current) for (const entry of await current.store.listBlobs()) files[entry.path] = entry
      for (const entry of frame.entries) if (entry.type === 'write') files[entry.path] = { path: entry.path, bytes: entry.bytes }
      return { entries: Object.values(files), store: current?.store ?? null }
    } catch (error) {
      this.errorDetail = error instanceof Error ? error.message : String(error)
      this.notice = 'pack.exportFailed'
      return null
    }
  }

  /** Where the pack goes: a file the user chose for much media, else the pieces are kept for a download. */
  private async packTarget(fileName: string, blobBytes: number): Promise<PackTarget | 'cancelled' | 'noRoom'> {
    if (blobBytes >= PACK_TO_FILE_BYTES && this.deps.saveFile) {
      const chosen = await this.deps.saveFile(fileName)
      if (chosen === 'cancelled') return 'cancelled'
      if (chosen) return chosen
    }
    // A download is kept by the browser (on disk when it is big) as the pieces come; check there is room for it.
    if (blobBytes >= PACK_TO_FILE_BYTES && this.deps.freeSpace) {
      const free = await this.deps.freeSpace()
      if (free !== null && free < blobBytes) return 'noRoom'
    }
    let kept = new Blob([], { type: 'application/zip' })
    return {
      write: async (chunk) => {
        kept = new Blob([kept, chunk as Uint8Array<ArrayBuffer>], { type: 'application/zip' })
      },
      close: async () => this.deps.download(fileName, kept),
      abort: async () => {
        kept = new Blob([])
      },
    }
  }

  private async writePack(prepared: { entries: FolderEntry[]; store: ProjectStore | null }, target: PackTarget, abort: AbortController): Promise<boolean> {
    this.exporting = 0
    const { store } = prepared
    let written = 0
    try {
      const io = {
        sink: {
          write: async (chunk: Uint8Array) => {
            await target.write(chunk)
            written += chunk.length
          },
        },
        signal: abort.signal,
        onProgress: (fraction: number) => (this.exporting = fraction),
      }
      // The store's Worker deflates when the folder is read there (#685); else the page's core does, reading the blobs through the store.
      const packed = store?.packExport
        ? await store.packExport(prepared.entries, io)
        : await this.host.packExport(prepared.entries, {
            ...io,
            read: (path, start, end) => (store ? store.read(path, start, end) : Promise.reject(new Error(`${path}: no folder to read from`))),
          })
      if (packed.status === 'ok') {
        await target.close()
        // A pack this page could not open again (where the browser keeps no projects, `MAX_PACK_BYTES`): it opens elsewhere,
        // so it is said, not refused.
        const tooBigHere = !this.deps.browser.available && written > MAX_PACK_BYTES
        this.notice = packed.missingBlobs > 0 ? 'pack.missingMedia' : tooBigHere ? 'pack.tooBigForPage' : null
        return true
      }
      await target.abort().catch(() => undefined)
      if (packed.status === 'failed') {
        this.errorDetail = packed.message
        this.notice = 'pack.exportFailed'
      } else {
        this.notice = 'pack.exportCancelled'
      }
      return false
    } catch (error) {
      await target.abort().catch(() => undefined)
      this.errorDetail = error instanceof Error ? error.message : String(error)
      this.notice = 'pack.exportFailed'
      return false
    } finally {
      this.exporting = null
    }
  }

  /** Import: the user picks a `.faulkpack`; its project is kept in the browser's storage and opened. Call straight from a click. */
  async importPack(): Promise<boolean> {
    if (this.needsGesture()) return false
    const file = await this.deps.pickPack()
    if (!file) return false
    const abort = this.beginImport()
    try {
      return await this.enqueue(() => this.importFile(file, abort.signal))
    } finally {
      this.endImport(abort)
    }
  }

  /** Stops the `.faulkpack` imports under way and those waiting their turn (`project.cancel_import_pack`), until their project starts opening. */
  cancelImport(): ImportCancel {
    let cancelled = 0
    let tooLate = 0
    for (const run of this.importRuns.values()) {
      if (run.committed) tooLate++
      else if (!run.abort.signal.aborted) {
        run.abort.abort()
        cancelled++
      }
    }
    return { cancelled, tooLate }
  }

  /** An import from the moment it is asked for: `cancelImport` reaches it while it waits, downloads or reads. */
  private beginImport(): AbortController {
    const abort = new AbortController()
    this.importRuns.set(abort.signal, { abort, committed: false })
    return abort
  }

  private endImport(abort: AbortController) {
    this.importRuns.delete(abort.signal)
  }

  /**
   * The point of no return of an import: its project starts to open. From here `cancelImport` leaves it alone and says it
   * came too late. `false` when it was cancelled before (nothing is committed then). No `await` between the caller's last
   * look at the signal and this, so a cancel cannot slip in between.
   */
  private commitImport(signal: AbortSignal): boolean {
    if (signal.aborted) return false
    const run = this.importRuns.get(signal)
    if (run) run.committed = true
    return true
  }

  private async importFile(file: File, signal: AbortSignal): Promise<boolean> {
    const name = file.name.replace(/\.faulkpack$/i, '') || 'pack'
    if (signal.aborted) {
      this.notice = 'pack.importCancelled'
      return false
    }
    // Into the browser's storage in pieces (#682): a pack of any size, read by the store's Worker.
    if (this.deps.browser.available) return this.importStreamed(file, name, {}, signal)
    // The tab alone (no browser storage): the project lives in its memory, and so does the pack.
    if (file.size > MAX_PACK_BYTES) {
      this.notice = 'pack.tooBig'
      return false
    }
    // Read whole, in pieces so that a cancel stops the read itself (#735); the banner shows it with its Cancel button.
    this.importing = 0
    let bytes: Uint8Array | null
    try {
      bytes = await readWhole(file, signal, (fraction) => (this.importing = fraction))
    } catch (error) {
      this.errorDetail = error instanceof Error ? error.message : String(error)
      this.notice = 'pack.invalid'
      return false
    } finally {
      this.importing = null
    }
    if (!bytes) {
      this.notice = 'pack.importCancelled'
      return false
    }
    return this.importBytes(bytes, name, {}, signal)
  }

  /**
   * Open a `.faulkpack` hosted at `url` (`project.open_url`): the page checks the link (see `checkOpenUrl`),
   * downloads it without credentials and with the size guard, then it goes the way of an imported file. Where the
   * browser keeps projects, the download is written as it comes into the new project's directory, then read from
   * there (#682); else it is held in memory, within `MAX_PACK_BYTES`. A pack whose project the browser keeps already
   * opens that copy instead of a second one (the same link clicked twice). The current project is never touched
   * before the pack is whole and valid. Downloads run one after the other, outside the storage's queue; the import
   * itself joins the queue. `allowed`: as for `openRecent`.
   */
  openUrl(url: string, allowed = false): Promise<boolean> {
    // A cancel reaches it from now on, while it waits behind another download too.
    const abort = this.beginImport()
    const run = this.downloads.then(() => this.fetchAndOpen(url, allowed, abort)).finally(() => this.endImport(abort))
    this.downloads = run.catch(() => undefined)
    return run
  }

  private async fetchAndOpen(raw: string, allowed: boolean, abort: AbortController): Promise<boolean> {
    const checked = checkOpenUrl(raw, this.deps.pageHostname())
    if (checked.status !== 'ok') {
      this.notice = 'openUrl.refused'
      return false
    }
    this.downloadFailed = null
    if (abort.signal.aborted) {
      this.notice = 'pack.importCancelled'
      return false
    }
    {
      if (this.deps.browser.available) return await this.downloadInto(checked, allowed, abort.signal)
      let download: Awaited<ReturnType<typeof downloadPack>>
      this.downloading = checked.host
      try {
        download = await downloadPack(checked.url, {
          fetch: this.deps.fetch,
          max: MAX_PACK_BYTES,
          pageHostname: this.deps.pageHostname(),
          signal: abort.signal,
        })
      } finally {
        this.downloading = null
      }
      if (download.status !== 'ok') {
        this.downloadRefused(download, checked.host)
        return false
      }
      const bytes = download.bytes
      return await this.enqueue(() => {
        if (abort.signal.aborted) {
          this.notice = 'pack.importCancelled'
          return Promise.resolve(false)
        }
        return this.importBytes(bytes, packNameOf(checked.url), { reuseKept: true, allowed }, abort.signal)
      })
    }
  }

  /** What the page says of a download that did not end with a pack. */
  private downloadRefused(download: { status: 'unreachable'; detail: string } | { status: 'tooBig' } | { status: 'cancelled' }, host: string) {
    if (download.status === 'unreachable') {
      this.errorDetail = download.detail
      this.downloadFailed = host
    } else {
      this.notice = download.status === 'tooBig' ? 'pack.tooBig' : 'pack.importCancelled'
    }
  }

  /**
   * The download of a link into a new directory of the browser's storage (its file `DOWNLOAD_PATH`, written in pieces
   * as it comes), then the import from there in the storage's queue.
   */
  private async downloadInto(checked: { url: URL; host: string }, allowed: boolean, signal: AbortSignal): Promise<boolean> {
    let target: ImportDir
    try {
      target = await this.importDir()
    } catch (error) {
      this.importFailed(error)
      return false
    }
    this.downloading = checked.host
    let streamed: Streamed
    try {
      const file = await target.store.writer(DOWNLOAD_PATH)
      // The pieces of the answer, gathered into pieces of `DOWNLOAD_PIECE_BYTES` for the Worker.
      let piece = new Uint8Array(DOWNLOAD_PIECE_BYTES)
      let filled = 0
      const flush = async () => {
        if (filled === 0) return
        const full = piece.subarray(0, filled)
        piece = new Uint8Array(DOWNLOAD_PIECE_BYTES)
        filled = 0
        await file.write(full.slice())
      }
      try {
        streamed = await streamPack(checked.url, {
          fetch: this.deps.fetch,
          max: MAX_STREAMED_PACK_BYTES,
          onProgress: (received, announced) => (this.downloadProgress = announced ? Math.min(1, received / announced) : null),
          pageHostname: this.deps.pageHostname(),
          signal,
          sink: {
            write: async (chunk) => {
              let at = 0
              while (at < chunk.length) {
                const n = Math.min(chunk.length - at, piece.length - filled)
                piece.set(chunk.subarray(at, at + n), filled)
                filled += n
                at += n
                if (filled === piece.length) await flush()
              }
            },
          },
        })
        if (streamed.status === 'ok') {
          await flush()
          await file.close()
        } else {
          await file.abort()
        }
      } catch (error) {
        await file.abort()
        throw error
      }
    } catch (error) {
      await this.dropImportDir(target)
      this.importFailed(error)
      return false
    } finally {
      this.downloading = null
      this.downloadProgress = null
    }
    if (streamed.status !== 'ok') {
      await this.dropImportDir(target)
      this.downloadRefused(streamed, checked.host)
      return false
    }
    return this.enqueue(() => this.importStreamed(DOWNLOAD_PATH, packNameOf(checked.url), { reuseKept: true, allowed }, signal, target))
  }

  /** A new directory of the browser's storage for a pack, locked (see `dirLock`), and its store. Throws what the browser refused. */
  private async importDir(): Promise<ImportDir> {
    const dir = crypto.randomUUID()
    const release = await this.deps.lock(dirLock(dir))
    try {
      const handle = await this.deps.browser.create(dir)
      const store = await this.deps.openStore(handle)
      return { dir, handle, store, release }
    } catch (error) {
      await this.deps.browser.remove(dir).catch(() => undefined)
      release?.()
      throw error
    }
  }

  /** The directory made for a pack goes, with whatever was written into it. */
  private async dropImportDir(target: ImportDir) {
    target.store.close()
    await this.deps.browser.remove(target.dir).catch(() => undefined)
    target.release?.()
  }

  /** An import cancelled once its files were written: the directory goes all the same. */
  private async cancelledAfterWrite(target: ImportDir): Promise<boolean> {
    await this.dropImportDir(target)
    this.notice = 'pack.importCancelled'
    return false
  }

  /** What the page says of an import the browser's storage refused. */
  private importFailed(error: unknown) {
    this.errorDetail = error instanceof Error ? error.message : String(error)
    this.notice = error instanceof StoreError && error.code === 'full' ? 'storage.browserFull' : 'pack.writeFailed'
  }

  /**
   * A pack read in pieces into a new directory of the browser's storage by the store's Worker (#682): first its project's own
   * files alone, to know the project (one the page has already is not imported again), then the whole pack, its blobs written
   * as they come; then its own files are written, `project.json` last, and the project opens. `source`: the file, or the path
   * of the pack in `given`'s directory (a download).
   */
  private async importStreamed(
    source: Blob | string,
    name: string,
    options: { reuseKept?: boolean; allowed?: boolean },
    signal: AbortSignal,
    given?: ImportDir,
  ): Promise<boolean> {
    const allowed = options.allowed ?? false
    let target: ImportDir
    if (given) {
      target = given
    } else {
      try {
        target = await this.importDir()
      } catch (error) {
        this.importFailed(error)
        return false
      }
    }
    const store = target.store
    const stop = async (key: MessageKey, detail?: string) => {
      await this.dropImportDir(target)
      if (detail !== undefined) this.errorDetail = detail
      this.notice = key
      return false
    }
    if (signal.aborted) return stop('pack.importCancelled')
    // The pack's blobs need about its size again (a download is there already, about as big).
    this.importing = 0
    let files: { path: string; bytes: Uint8Array }[]
    try {
      // What the project of the pack is, before any of its media is read.
      const head = await store.unpack(source, { signal, textOnly: true })
      if (head.status === 'cancelled') return await stop('pack.importCancelled')
      if (head.status === 'failed') return await stop('pack.invalid', head.message)
      // The blobs take what the pack's directory declares (a download is counted in the browser's use already).
      const free = this.deps.freeSpace ? await this.deps.freeSpace() : null
      if (free !== null && free < head.blobBytes) return await stop('storage.browserFull')
      // A cancel holds until a project starts opening, the copy the browser keeps too.
      if (signal.aborted) return await stop('pack.importCancelled')
      const known = await this.knownProject(head.files, name, options)
      if (known) {
        // Opening the copy the browser keeps is the point of no return too.
        if (known.open !== undefined && !this.commitImport(signal)) return await stop('pack.importCancelled')
        await this.dropImportDir(target)
        return known.open !== undefined ? this.openKept(known.open, allowed) : known.result
      }
      const unpacked = await store.unpack(source, { signal, onProgress: (fraction) => (this.importing = fraction) })
      if (unpacked.status === 'cancelled') return await stop('pack.importCancelled')
      if (unpacked.status === 'failed') return await stop('pack.invalid', unpacked.message)
      files = unpacked.files
      if (signal.aborted) return await stop('pack.importCancelled')
      // The downloaded pack goes first, then the project's own files, `project.json` last: the directory is a project
      // only once it is whole.
      const rank = (path: string) => (path === 'project.json' ? 1 : 0)
      const writes = [...files].sort((a, b) => rank(a.path) - rank(b.path)).map((f): SaveEntry => ({ type: 'write', path: f.path, bytes: f.bytes }))
      await target.store.apply([...(typeof source === 'string' ? [{ type: 'remove' as const, path: source }] : []), ...writes])
    } catch (error) {
      await this.dropImportDir(target)
      this.importFailed(error)
      return false
    } finally {
      this.importing = null
    }
    // The last moment a cancel reaches it: once the project starts opening, it opens.
    if (!this.commitImport(signal)) return this.cancelledAfterWrite(target)
    target.store.close()
    target.release?.()
    const opened = await this.openHandle(target.handle, { kind: 'browser', dir: target.dir, allowed })
    // Declined (the question before replacing a project) or refused: the copy made for it goes.
    if (!opened) await this.deps.browser.remove(target.dir).catch(() => undefined)
    else this.persistOnce()
    await this.refreshBrowserProjects()
    return opened
  }

  /**
   * Whether the project of a pack is one the page has already (the pack keeps its identity, so what it gives back is the
   * project it was made from; a project of that identity that is here, open or kept in the browser, is never overwritten by
   * it). `null`: a new project, import it. Else what to do instead: the import's answer, or the kept copy to open.
   */
  private async knownProject(
    files: { path: string; bytes: Uint8Array }[],
    name: string,
    options: { reuseKept?: boolean },
  ): Promise<{ result: boolean; open?: undefined } | { open: string } | null> {
    const id = projectIdOf(files)
    if (!id) return null
    const kept = this.deps.browser.available ? (await this.deps.browser.list()).projects.find((p) => p.id === id) : undefined
    if (options.reuseKept && this.host.projectId() === id) {
      this.notice = 'openUrl.alreadyOpen'
      return { result: true }
    }
    if (options.reuseKept && kept) return { open: kept.dir }
    if (kept || this.host.projectId() === id) {
      // Names the project and, when it is kept in the browser and not the one open, offers to open it.
      const open = this.host.projectId() === id ? null : (kept?.dir ?? null)
      this.notice = open === null ? 'pack.alreadyOpen' : 'pack.alreadyHere'
      this.noticeVars = { name: kept?.name ?? name }
      this.noticeOpen = open
      return { result: false }
    }
    return null
  }

  /** The pack's bytes, checked by the core, kept in a new directory of the browser's storage and opened. */
  private async importBytes(bytes: Uint8Array, name: string, options: { reuseKept?: boolean; allowed?: boolean }, signal: AbortSignal): Promise<boolean> {
    const allowed = options.allowed ?? false
    // Read and checked before anything is touched: a file that is no pack changes nothing.
    let unpacked: PackImport
    try {
      unpacked = await this.host.packImport(bytes)
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
    // A cancel holds until the project starts opening (#735).
    if (signal.aborted) {
      this.notice = 'pack.importCancelled'
      return false
    }
    const known = await this.knownProject(files, name, options)
    if (known) {
      if (known.open === undefined) return known.result
      if (!this.commitImport(signal)) {
        this.notice = 'pack.importCancelled'
        return false
      }
      return this.openKept(known.open, allowed)
    }
    if (!this.deps.browser.available) {
      if (!this.commitImport(signal)) {
        this.notice = 'pack.importCancelled'
        return false
      }
      return this.openImported(name, files, allowed)
    }
    // (Not reached from the page, which reads a pack in pieces where the browser keeps projects.) Into a directory of the browser's storage first, as a pack is written into an empty folder: the
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
      this.importFailed(error)
      return false
    }
    releaseDir?.()
    const opened = await this.openHandle(handle, { kind: 'browser', dir, allowed })
    // Declined (the question before replacing a project) or refused: the copy made for it goes.
    if (!opened) await this.deps.browser.remove(dir).catch(() => undefined)
    else this.persistOnce()
    await this.refreshBrowserProjects()
    return opened
  }

  /** A link opened again: the copy the browser keeps (with whatever was changed in it since) opens, and the page says so. */
  private async openKept(dir: string, allowed: boolean): Promise<boolean> {
    const handle = await this.deps.browser.open(dir)
    if (!handle) {
      this.notice = 'storage.browserGone'
      return false
    }
    const opened = await this.openHandle(handle, { kind: 'browser', dir, allowed })
    if (opened) this.notice = 'openUrl.keptCopy'
    await this.refreshBrowserProjects()
    return opened
  }

  /** Where the browser keeps nothing: the pack's project opens in the tab alone. */
  private async openImported(name: string, files: { path: string; bytes: Uint8Array }[], allowed: boolean): Promise<boolean> {
    if (!(await this.makeRoom('open', allowed))) return false
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
