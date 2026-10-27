// The browser's side of the project store (SV-2): the folder picker, the list of recent projects
// (the folders' handles live in IndexedDB), the permission calls and the single-writer lock. Each
// is a small seam, so the controller (`storage.svelte.ts`) is tested without a dialog.
import { OpfsProjects } from './opfs'
import { openStore } from './rpc'
import type { StorageDeps } from './storage.svelte'
import { MAX_RECENT, type FolderHandle, type FolderPicker, type RecentProject, type RecentProjects } from './types'

type PickerWindow = { showDirectoryPicker?: (options: { id: string; mode: 'readwrite' }) => Promise<FolderHandle> }

/** `showDirectoryPicker`: Chromium on a computer. Firefox and Safari have none (their store is SV-3). */
export const browserPicker: FolderPicker = {
  get supported() {
    return typeof window !== 'undefined' && typeof (window as PickerWindow).showDirectoryPicker === 'function'
  },
  async pick() {
    try {
      const handle = await (window as PickerWindow).showDirectoryPicker!({ id: 'faulk-project', mode: 'readwrite' })
      return { status: 'chosen', handle }
    } catch (error) {
      if (error instanceof DOMException && error.name === 'AbortError') return { status: 'cancelled' }
      // A folder the browser does not allow (a system folder, a disk root) throws a SecurityError;
      // a page that has no click behind the call, a NotAllowedError.
      return { status: 'refused' }
    }
  },
}

/** The permission on a folder now; where the browser has no such call (a folder it cannot revoke), granted. */
export async function permissionOf(handle: FolderHandle): Promise<PermissionState> {
  if (typeof handle.queryPermission !== 'function') return 'granted'
  try {
    return await handle.queryPermission({ mode: 'readwrite' })
  } catch {
    return 'denied'
  }
}

/** Asks the user for the permission (a prompt). Call straight from a click. */
export async function askPermission(handle: FolderHandle): Promise<PermissionState> {
  if (typeof handle.requestPermission !== 'function') return 'granted'
  try {
    return await handle.requestPermission({ mode: 'readwrite' })
  } catch {
    return 'denied'
  }
}

const DB = 'faulk-projects'
const STORE = 'recent'

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB, 1)
    request.onupgradeneeded = () => request.result.createObjectStore(STORE, { keyPath: 'id' })
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error ?? new Error('IndexedDB request failed'))
  })
}

function done<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error ?? new Error('IndexedDB request failed'))
  })
}

/**
 * The recent projects, in IndexedDB. Everything is best effort: a private window or a browser
 * that refuses IndexedDB gives an empty list, and a project still saves; it is only not offered
 * again.
 */
export const browserRecent: RecentProjects = {
  async list() {
    try {
      const db = await openDb()
      const all = (await done(db.transaction(STORE).objectStore(STORE).getAll())) as RecentProject[]
      db.close()
      return all.sort((a, b) => b.at - a.at)
    } catch {
      return []
    }
  },
  async remember(handle) {
    try {
      const known = await this.list()
      let id: string = crypto.randomUUID()
      for (const entry of known) {
        if (await entry.handle.isSameEntry(handle).catch(() => false)) id = entry.id
      }
      const db = await openDb()
      const store = db.transaction(STORE, 'readwrite').objectStore(STORE)
      await done(store.put({ id, name: handle.name, handle, at: Date.now() } satisfies RecentProject))
      // Keep the newest few.
      const all = ((await done(store.getAll())) as RecentProject[]).sort((a, b) => b.at - a.at)
      for (const old of all.slice(MAX_RECENT)) await done(store.delete(old.id))
      db.close()
    } catch {
      // Not remembered; the project is saved all the same.
    }
  },
  async forget(id) {
    try {
      const db = await openDb()
      await done(db.transaction(STORE, 'readwrite').objectStore(STORE).delete(id))
      db.close()
    } catch {
      // Nothing to forget.
    }
  },
}

/**
 * Holds the right to write a project for as long as the page keeps it: one tab saves a given
 * project at a time (two tabs on one folder would overwrite each other's files, each believing
 * its own copy is the folder's). A Web Lock named after the project's identity, so it holds
 * across tabs and across folders that share the identity. Returns a release function, or
 * `null` when another tab holds the project. Where Web Locks do not exist there is no lock.
 */
export async function lockProject(id: string): Promise<(() => void) | null> {
  const locks = typeof navigator === 'undefined' ? undefined : navigator.locks
  if (!locks) return () => undefined
  return new Promise((resolve) => {
    void locks.request(`faulk.project.${id}`, { ifAvailable: true }, (lock) => {
      if (!lock) {
        resolve(null)
        return undefined
      }
      // The lock is held until this promise settles: when the page releases it, or goes away.
      return new Promise<void>((release) => resolve(() => release()))
    })
  })
}

type ObserverRecord = { type: string }
type ObserverCtor = new (callback: (records: ObserverRecord[]) => void) => {
  observe(handle: FileSystemHandle, options?: { recursive?: boolean }): Promise<void>
  disconnect(): void
}

/**
 * Calls back when the folder may have changed: `FileSystemObserver` where it exists (Chrome and
 * Edge 133), otherwise never; the page then looks again when it comes back to the front.
 */
export function watchFolder(handle: FolderHandle, onChange: () => void): () => void {
  const Observer = (globalThis as { FileSystemObserver?: ObserverCtor }).FileSystemObserver
  if (!Observer) return () => undefined
  try {
    const observer = new Observer(() => onChange())
    observer.observe(handle, { recursive: true }).catch(() => undefined)
    return () => observer.disconnect()
  } catch {
    return () => undefined
  }
}

/** The file chooser for a `.faulkpack`. Call straight from a click: the browser wants the gesture. `null` when cancelled. */
export function pickPackFile(): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = '.faulkpack,application/zip'
    input.hidden = true
    const done = (file: File | null) => {
      input.remove()
      resolve(file)
    }
    input.addEventListener('change', () => done(input.files?.[0] ?? null))
    input.addEventListener('cancel', () => done(null))
    document.body.append(input)
    input.click()
  })
}

/** Hands a file to the user: a download named `name`. */
export function downloadFile(name: string, blob: Blob): void {
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = name
  link.hidden = true
  document.body.append(link)
  link.click()
  link.remove()
  // The browser starts the download from the click; the address can go a moment later.
  setTimeout(() => URL.revokeObjectURL(url), 60_000)
}

/** The controller's browser: the real picker, IndexedDB, Web Locks, the OPFS and a Worker per page. */
export const browserDeps: StorageDeps = {
  picker: browserPicker,
  recent: browserRecent,
  openStore,
  permission: permissionOf,
  askPermission,
  lock: lockProject,
  watch: watchFolder,
  now: () => Date.now(),
  browser: new OpfsProjects(),
  pickPack: pickPackFile,
  download: downloadFile,
}
