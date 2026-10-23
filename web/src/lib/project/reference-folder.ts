// Test helper (Node only): the reference project folder of the folder format, read from disk the way
// the page's store will hand a folder over (`FolderEntry`, bytes for what opening reads).
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { needsBytes, type FolderEntry } from './core'

/** `crates/project/tests/reference/song`: two tracks, a scene, clips, a lane stored as a blob. */
export const SONG_FOLDER = join(process.cwd(), '..', 'crates', 'project', 'tests', 'reference', 'song')

/** The entries of a folder on disk. */
export function readFolder(dir: string, base = ''): FolderEntry[] {
  return readdirSync(dir).flatMap((name): FolderEntry[] => {
    const path = join(dir, name)
    const relative = base ? `${base}/${name}` : name
    if (statSync(path).isDirectory()) return readFolder(path, relative)
    const bytes = new Uint8Array(readFileSync(path))
    return needsBytes(relative, bytes.subarray(0, 8)) ? [{ path: relative, bytes }] : [{ path: relative, size: bytes.length }]
  })
}
