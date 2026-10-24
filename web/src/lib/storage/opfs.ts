// The browser's own file system (OPFS) as a home for projects (SV-3): a draft before the user has
// chosen a folder, and the project itself where the browser has no folder to offer (Firefox,
// Safari). A directory of the OPFS is a `FileSystemDirectoryHandle` like the one the folder
// picker gives, so the project store (`FolderBackend`, in its Worker) runs on it unchanged, with
// the same atomic writes; this file is only where the directories are, how they are listed, and
// what the browser says about keeping them.
import { SKIPPED_DIRS, StoreError, type BrowserProject, type BrowserProjects, type FolderHandle } from './types'

/** Where the projects are: `faulk/browser/<dir>/` in the origin's private file system. */
const PARTS = ['faulk', 'browser'] as const

/** Whether the page can keep projects in the OPFS: the API exists and files can be written atomically (`createWritable`). */
export function opfsSupported(): boolean {
  if (typeof navigator === 'undefined' || typeof navigator.storage?.getDirectory !== 'function') return false
  const file = (globalThis as { FileSystemFileHandle?: { prototype: { createWritable?: unknown } } }).FileSystemFileHandle
  return typeof file?.prototype.createWritable === 'function'
}

async function root(create: boolean): Promise<FileSystemDirectoryHandle> {
  let dir = await navigator.storage.getDirectory()
  for (const part of PARTS) dir = await dir.getDirectoryHandle(part, { create })
  return dir
}

type Listable = { values(): AsyncIterable<FileSystemHandle> }
const children = (dir: FileSystemDirectoryHandle): AsyncIterable<FileSystemHandle> => (dir as unknown as Listable).values()

/** When a project last changed: the newest file of its folder, `blobs/` apart (named by content, never rewritten). */
async function newestModified(dir: FileSystemDirectoryHandle, depth = 0): Promise<number> {
  let newest = 0
  for await (const entry of children(dir)) {
    if (entry.kind === 'file') {
      newest = Math.max(newest, (await (entry as FileSystemFileHandle).getFile()).lastModified)
    } else if (depth < 4 && entry.name !== 'blobs' && !SKIPPED_DIRS.includes(entry.name)) {
      newest = Math.max(newest, await newestModified(entry as FileSystemDirectoryHandle, depth + 1))
    }
  }
  return newest
}

/**
 * What the page lists of a project without opening it: its identity and name, from `project.json`.
 * `'incomplete'` for a directory with no `project.json` at all (a project being made, or one a
 * closed tab left half made: `project.json` is the last file a first write puts down); `null` for
 * one whose `project.json` does not read, which is left alone, neither offered nor swept.
 */
async function describe(dir: string, handle: FileSystemDirectoryHandle): Promise<BrowserProject | 'incomplete' | null> {
  let file: File
  try {
    file = await (await handle.getFileHandle('project.json')).getFile()
  } catch (error) {
    return error instanceof DOMException && error.name === 'NotFoundError' ? 'incomplete' : null
  }
  try {
    const json = JSON.parse(await file.text()) as { id?: unknown; name?: unknown }
    if (typeof json.id !== 'string') return null
    return { dir, id: json.id, name: typeof json.name === 'string' && json.name ? json.name : dir, modified: await newestModified(handle) }
  } catch {
    return null
  }
}

/**
 * The OPFS as the page's browser projects. `available` is false where the OPFS cannot be used
 * (no API, no `createWritable`, a private window that refuses it): the project then stays in the
 * tab, with the warning before leaving, as before SV-3. It also turns false when the OPFS
 * refuses its first directory, so a browser that only fails late is not asked again.
 */
export class OpfsProjects implements BrowserProjects {
  // A browser check sets `__faulkNoBrowserStorage` to look at the page as it is where the OPFS cannot be used
  // (the checks that simulate the folder picker with the OPFS itself need it for that, not for drafts).
  private usable = opfsSupported() && !(globalThis as { __faulkNoBrowserStorage?: boolean }).__faulkNoBrowserStorage

  get available(): boolean {
    return this.usable
  }

  async list(): Promise<{ projects: BrowserProject[]; incomplete: string[] }> {
    if (!this.usable) return { projects: [], incomplete: [] }
    try {
      const projects: BrowserProject[] = []
      const incomplete: string[] = []
      for await (const entry of children(await root(true))) {
        if (entry.kind !== 'directory') continue
        const project = await describe(entry.name, entry as FileSystemDirectoryHandle)
        if (project === 'incomplete') incomplete.push(entry.name)
        else if (project) projects.push(project)
      }
      return { projects: projects.sort((a, b) => b.modified - a.modified), incomplete }
    } catch {
      return { projects: [], incomplete: [] }
    }
  }

  async create(dir: string): Promise<FolderHandle> {
    try {
      return await (await root(true)).getDirectoryHandle(dir, { create: true })
    } catch (error) {
      const name = error instanceof DOMException ? error.name : ''
      if (name === 'QuotaExceededError') throw new StoreError('full', 'the browser has no room for a new project')
      // The OPFS refuses to open at all (a private window, a policy): nothing will work, so say so once.
      this.usable = false
      throw new StoreError('io', `the browser storage is not available: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  async open(dir: string): Promise<FolderHandle | null> {
    if (!this.usable) return null
    try {
      return await (await root(false)).getDirectoryHandle(dir)
    } catch {
      return null
    }
  }

  async remove(dir: string): Promise<void> {
    try {
      await (await root(false)).removeEntry(dir, { recursive: true })
    } catch (error) {
      if (error instanceof DOMException && error.name === 'NotFoundError') return
      throw new StoreError('io', `the project could not be deleted: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  async persist(): Promise<boolean> {
    try {
      const storage = navigator.storage
      if (typeof storage.persisted === 'function' && (await storage.persisted())) return true
      return typeof storage.persist === 'function' ? await storage.persist() : false
    } catch {
      return false
    }
  }
}
