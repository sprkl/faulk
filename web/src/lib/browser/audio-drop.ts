/**
 * Audio files dropped on the empty area of the track list (#190, AU-4 #113): a new audio track takes them. The page
 * makes the track, selects it, shows its Clips panel and imports the files into it with AU-1's import, as its "Import
 * audio…" button does. The files come from the browser's own `DataTransfer`.
 *
 * Not one undo group (lead's decision, #113): the import is asynchronous (each file is read, hashed, decoded and kept in
 * the project), and a history group held open over that work would take in whatever else the person does meanwhile.
 * So the track is one history entry and each imported file one more, as an import into an existing track already is:
 * Undo takes the clips away first, then the track. Files that are not audio by their type or their name make no track,
 * and the person is told so; a file that looks like audio but cannot be read is refused by the import, in the Clips
 * panel, like one chosen with the button.
 *
 * Files dropped on an audio track itself (AU-6, #115) go where they were dropped: on its lane in the arrangement
 * (`placeOnLane`) or on an empty cell of its column in the grid (`placeInGrid`). The same asynchronous import, and each
 * file's clip made by a command once the file is read.
 */

import type { MessageKey } from '../i18n'
import type { AudioMedia, Command } from '../generated/protocol'
import { importCommand, type ImportPlacer } from '../audio/import/audio-import.svelte'

/** What a drop needs of the page. */
export interface AudioDropHost {
  /** Makes an audio track, selects it and shows its Clips panel; its id, or null when it could not be made. */
  createAudioTrack(): string | null
  /** Imports the files into the track (AU-1), one history entry per file; resolves with the clips made. */
  importFiles(track: string, files: File[]): Promise<string[]>
}

/** The extensions of the formats the import reads (AU-1), for a file the browser gives no `audio/` type. */
const AUDIO_EXTENSIONS = new Set(['wav', 'wave', 'aif', 'aiff', 'aifc', 'caf', 'flac', 'mp3', 'm4a', 'aac', 'mp4', 'ogg', 'oga'])

/** Whether a dropped file looks like audio: its type, else its extension. */
export function isAudioFile(file: Pick<File, 'name' | 'type'>): boolean {
  if (file.type.startsWith('audio/')) return true
  const dot = file.name.lastIndexOf('.')
  return dot >= 0 && AUDIO_EXTENSIONS.has(file.name.slice(dot + 1).toLowerCase())
}

/** What the placers below need of the page: the dispatcher, a history group, and what the project holds now. */
export interface PlaceHost {
  run(command: Command): boolean
  /** Runs `body` as one history group. */
  group<T>(body: () => T): T
  /** The clips of a track, now. */
  clipsOf(track: string): string[]
}

/** The commands a lane drop needs of the page, besides `PlaceHost`. */
export interface LaneHost extends PlaceHost {
  /** Whether the ticks `[from, to)` of the lane are free now. */
  free(from: number, to: number): boolean
  /** The ticks a media takes at the project's tempo. */
  ticksOf(media: AudioMedia): number
}

/**
 * Files dropped on an audio track's lane (AU-6): each one's clip goes in the arrangement, the first from `start`, each
 * next one where the last one ends. Where its span is free, `clip.import_audio` makes and places it (one command);
 * where it would cover another placement, the clip is made in the pool and `arrangement.place` puts it there (it carves
 * what it covers, as every placement put down does), both in one history group. Decided when each file's clip is made,
 * after its import: the arrangement may have changed while the file was read.
 */
export function placeOnLane(host: LaneHost, track: string, start: number): ImportPlacer {
  let next = Math.max(0, Math.round(start))
  return (media) => {
    const at = next
    const length = host.ticksOf(media)
    next = at + length
    if (host.free(at, at + length)) return host.run({ type: 'clip.import_audio', track, media: media.hash, name: null, at: { at: 'arrangement', start: at } })
    return host.group(() => madeThen(host, track, media, (clip) => host.run({ type: 'arrangement.place', clip, start: at })))
  }
}

/**
 * Files dropped on an empty cell of an audio track in the grid (AU-6): the first goes into that cell, each next one into
 * the next free cell below (`freeFrom` finds it, now), and a file with no free cell left into the track's pool.
 */
export function placeInGrid(host: PlaceHost, track: string, freeFrom: (index: number) => string | null): ImportPlacer {
  return (media, index) => {
    const scene = freeFrom(index)
    return host.run({ type: 'clip.import_audio', track, media: media.hash, name: null, at: scene ? { at: 'slot', scene } : null })
  }
}

/** Makes the clip of `media` in the track's pool, then hands its id to `then`. */
function madeThen(host: PlaceHost, track: string, media: AudioMedia, then: (clip: string) => boolean): boolean {
  const before = new Set(host.clipsOf(track))
  if (!host.run(importCommand(track, media.hash))) return false
  const made = host.clipsOf(track).find((id) => !before.has(id))
  return made !== undefined && then(made)
}

/**
 * Takes a drop of files: a new audio track and their import when at least one looks like audio. Returns what to tell
 * the person.
 */
export function dropAudioFiles(host: AudioDropHost, files: readonly File[]): MessageKey {
  const audio = files.filter(isAudioFile)
  if (audio.length === 0) return 'browser.audioNone'
  const track = host.createAudioTrack()
  if (!track) return 'browser.audioNoTrack'
  // The Clips panel shows the progress and any refusal; the promise carries nothing more to say here.
  void host.importFiles(track, audio).catch(() => undefined)
  return 'browser.audioImporting'
}
