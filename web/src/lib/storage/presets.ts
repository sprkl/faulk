// The user's preset library (#191): one preset file per preset, `faulk/presets/<id>.json` in the
// origin's private file system, in the format the core writes (`crates/project/src/preset`). The
// library is every project's, in Web and in Boost alike (`docs/architecture/03-stockage-local.md` § 5.9).
// The core holds the presets (`State::library`); this is only where their files are.
import { opfsSupported } from './opfs'

/** A file of the library: its id (the name without `.json`) and its text. */
export interface LibraryFile {
  id: string
  text: string
}

/** Where the library's files are. */
export interface LibraryDir {
  /** The directory, made when `create`; `null` when there is none (and none is made). */
  open(create: boolean): Promise<FileSystemDirectoryHandle | null>
}

/** The library in the OPFS, at `faulk/presets/`. */
export const opfsLibraryDir: LibraryDir = {
  async open(create) {
    try {
      let dir = await navigator.storage.getDirectory()
      for (const part of ['faulk', 'presets']) dir = await dir.getDirectoryHandle(part, { create })
      return dir
    } catch {
      return null
    }
  },
}

type Listable = { values(): AsyncIterable<FileSystemHandle> }
const children = (dir: FileSystemDirectoryHandle): AsyncIterable<FileSystemHandle> => (dir as unknown as Listable).values()

/** A preset id as the core makes them: a ULID. Anything else in the directory is not read and never removed. */
const ID = /^[0-7][0-9A-HJKMNP-TV-Z]{25}$/

/** The files of the library; writes and removes one file at a time. */
export class PresetLibrary {
  constructor(
    private readonly dir: LibraryDir = opfsLibraryDir,
    // A browser check sets `__faulkNoBrowserStorage` to look at the page as it is where the OPFS cannot be used.
    readonly available: boolean = opfsSupported() && !(globalThis as { __faulkNoBrowserStorage?: boolean }).__faulkNoBrowserStorage,
  ) {}

  private persistAsked = false

  /** Every preset file of the library (the files whose name is not a preset id are left alone). Empty when there is no library. */
  async read(): Promise<LibraryFile[]> {
    if (!this.available) return []
    const dir = await this.dir.open(false)
    if (!dir) return []
    const out: LibraryFile[] = []
    for await (const entry of children(dir)) {
      if (entry.kind !== 'file' || !entry.name.endsWith('.json')) continue
      const id = entry.name.slice(0, -'.json'.length)
      if (!ID.test(id)) continue
      try {
        out.push({ id, text: await (await (entry as FileSystemFileHandle).getFile()).text() })
      } catch {
        // A file that cannot be read now is left for the next read.
      }
    }
    return out
  }

  /**
   * Writes a preset's file. `createWritable` writes aside and replaces the file only when it is closed:
   * a tab closed during the write leaves the old file as it was. Throws when the storage refuses.
   */
  async write(id: string, bytes: Uint8Array): Promise<void> {
    if (!this.available) return
    if (!ID.test(id)) throw new Error(`not a preset id: ${id}`)
    const dir = await this.dir.open(true)
    if (!dir) throw new Error('the browser storage is not available')
    // The library is kept by the browser, which may evict it under pressure unless it is asked to keep it: once.
    if (!this.persistAsked) {
      this.persistAsked = true
      void navigator.storage?.persist?.().catch(() => false)
    }
    const file = await dir.getFileHandle(`${id}.json`, { create: true })
    const stream = await file.createWritable()
    try {
      await stream.write(bytes as Uint8Array<ArrayBuffer>)
      await stream.close()
    } catch (error) {
      await stream.abort().catch(() => {})
      throw error
    }
  }

  /** Removes a preset's file; nothing when it is not there. */
  async remove(id: string): Promise<void> {
    if (!this.available || !ID.test(id)) return
    const dir = await this.dir.open(false)
    if (!dir) return
    try {
      await dir.removeEntry(`${id}.json`)
    } catch (error) {
      if (!(error instanceof DOMException && error.name === 'NotFoundError')) throw error
    }
  }
}
