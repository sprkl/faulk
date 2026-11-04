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
 */

import type { MessageKey } from '../i18n'

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
