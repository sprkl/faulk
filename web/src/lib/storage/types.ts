// The browser's File System Access API, where `lib.dom` stops short, and what the project
// store (SV-2) says about a folder. The store is the one place that touches a folder; the rest of
// the page sees `ProjectStore` and `StoreError`.
import type { CacheReport, ImportOutcome, MediaRef } from '../audio/import/pipeline'
import type { FolderEntry, SaveEntry } from '../project/core'

/** `FileSystemHandle.queryPermission` and `requestPermission` (Chromium; not in `lib.dom`). */
export interface PermissionHandle {
  queryPermission(descriptor?: { mode?: 'read' | 'readwrite' }): Promise<PermissionState>
  requestPermission(descriptor?: { mode?: 'read' | 'readwrite' }): Promise<PermissionState>
}

/** A folder the user chose, as the page holds it. */
export type FolderHandle = FileSystemDirectoryHandle & Partial<PermissionHandle>

/**
 * What the page remembers of every file that matters to the project: `<size>:<modified>` by path
 * (everything but `.faulk/` and `.git/`). Two scans that differ say a file was written, whoever
 * wrote it; the page compares a scan with the stamps of its own writes to tell an external edit.
 */
export type Signature = Record<string, string>

/** Why a store call failed, in terms the page acts on. */
export type StoreErrorCode =
  /** The browser withdrew the right to write (or never gave it): ask again, with a click. */
  | 'permission'
  /** The disk or the browser's quota is full. */
  | 'full'
  /** A file or folder the call needs is not there (deleted behind our back). */
  | 'missing'
  /** Anything else the file system refused (a locked file, a failed close). */
  | 'io'

export class StoreError extends Error {
  constructor(
    readonly code: StoreErrorCode,
    message: string,
  ) {
    super(message)
    this.name = 'StoreError'
  }
}

/** The project folder as the page needs it: read it whole, read a range, write atomically, watch. */
export interface ProjectStore {
  /** The folder's name (what the user called it). */
  readonly name: string
  /** Whether the folder holds nothing (a `.DS_Store` or `Thumbs.db` does not count). */
  isEmpty(): Promise<boolean>
  /** Whether the folder is a Faulk project (it has a `project.json`). */
  hasProject(): Promise<boolean>
  /**
   * Every file `project.open` reads, with the stamps they had: the text files and the blobs of
   * dense automation lanes as bytes, the other blobs by size (`needsBytes`).
   */
  readFolder(): Promise<{ entries: FolderEntry[]; signature: Signature }>
  /** Every file of `blobs/`, whole (an export carries the media a save does not rewrite). */
  readBlobs(): Promise<{ path: string; bytes: Uint8Array }[]>
  /** Bytes `[start, end)` of a file, without reading the rest. */
  read(path: string, start?: number, end?: number): Promise<Uint8Array>
  /**
   * Applies the entries in order, each file replaced atomically: a file is whole old or whole new.
   * Stops at the first failure (the entries before it stay applied). Returns the stamps of the
   * files written, to compare later scans with.
   */
  apply(entries: SaveEntry[]): Promise<Signature>
  /** The folder's stamps now. */
  scan(): Promise<Signature>
  /** Copies the files of `blobs/` that this folder lacks from `source`'s (audio is not in a save). */
  copyBlobsFrom(source: ProjectStore): Promise<void>
  /**
   * Imports an audio file into the folder (AU-1, `lib/audio/import/pipeline.ts`): its bytes into
   * `blobs/<sha256>` (once, however often it is imported), its decoded audio into the PCM cache
   * when it is not read directly. Rejects with an `ImportError` (a file it cannot take, a cancel) or
   * a `StoreError` (the folder refused a write).
   */
  importAudio(file: File, onProgress?: (fraction: number) => void, signal?: AbortSignal): Promise<ImportOutcome>
  /** Makes again the PCM caches the folder lacks for these media (after opening a project). */
  ensureAudioCache(media: MediaRef[], onProgress?: (fraction: number) => void, signal?: AbortSignal): Promise<CacheReport>
  /** Removes the swap files an interrupted write left (`*.crswap`). Best effort. */
  cleanUp(): Promise<void>
  /** Releases what the store holds (its Worker's side). */
  close(): void
}

/** A project kept in the browser's own storage (the OPFS): a draft before a folder is chosen, or the project itself where there is no folder. */
export interface BrowserProject {
  /** Its directory (a random name, not the project's identity). */
  dir: string
  /** The project's identity, from its `project.json`. */
  id: string
  name: string
  /** When a file of it last changed, in ms. */
  modified: number
}

/** The browser's own storage as a home for projects (`opfs.ts`); a stand-in in tests. */
export interface BrowserProjects {
  /** Whether projects can be kept here at all (false: the project stays in the tab, as before drafts). */
  readonly available: boolean
  /**
   * The projects, newest change first, and the directories that hold no project yet: a creation
   * under way in another tab, or one a closed tab left behind (see `sweep`).
   */
  list(): Promise<{ projects: BrowserProject[]; incomplete: string[] }>
  /** Makes the directory `dir` for a new project. */
  create(dir: string): Promise<FolderHandle>
  open(dir: string): Promise<FolderHandle | null>
  /** Deletes a project's directory with everything in it; a directory already gone is not an error. */
  remove(dir: string): Promise<void>
  /** Asks the browser to keep the storage when it is short of room (`navigator.storage.persist()`). */
  persist(): Promise<boolean>
}

/** The folder picker: `showDirectoryPicker` behind a seam, so the page can be driven without a dialog. */
export interface FolderPicker {
  /** Whether this browser can open a folder at all (Chromium on a computer). */
  readonly supported: boolean
  /**
   * Asks the user for a folder. Must be called straight from a click: the browser wants the
   * gesture. `cancelled` when the user closed the dialog; `refused` when the browser does not
   * allow that folder (a system folder, the root of a disk).
   */
  pick(): Promise<{ status: 'chosen'; handle: FolderHandle } | { status: 'cancelled' } | { status: 'refused' }>
}

/** The projects the user opened or saved, newest first. */
export interface RecentProject {
  id: string
  name: string
  handle: FolderHandle
  /** When it was last opened or saved into, in ms. */
  at: number
}

export interface RecentProjects {
  list(): Promise<RecentProject[]>
  /** Puts the folder first (a folder already there moves up). */
  remember(handle: FolderHandle): Promise<void>
  forget(id: string): Promise<void>
}

/** The most recent projects kept. */
export const MAX_RECENT = 8

/** The root files that do not make a folder non-empty. */
export const IGNORED_FILES: readonly string[] = ['.DS_Store', 'Thumbs.db', 'desktop.ini']

/** The extension Chromium gives a file being written, beside its target. */
export const SWAP_SUFFIX = '.crswap'

/** The directories no scan or read looks into. */
export const SKIPPED_DIRS: readonly string[] = ['.git']
