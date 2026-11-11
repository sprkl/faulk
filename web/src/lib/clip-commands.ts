import type { Command, Note, TickLoop } from './generated/protocol'

/**
 * The commands of the piano roll: every edit of a clip is one of them, and a gesture sends one
 * batch (all or nothing, one history entry), never a command per pointer move.
 */

/** A clip in the arrangement from the first tick, one bar long (the project's default): the clip the roll creates for a track that has none. */
export const createClipAtStart = (track: string): Command => ({ type: 'clip.create', track, at: { at: 'arrangement', start: 0 } })
export const addNotes = (clip: string, notes: Note[]): Command => ({ type: 'clip.notes.add', clip, notes })
export const removeNotes = (clip: string, notes: Note[]): Command => ({ type: 'clip.notes.remove', clip, notes })
export const updateNotes = (clip: string, changes: { from: Note; to: Note }[]): Command => ({ type: 'clip.notes.update', clip, changes })
/** The start of notes toward a grid by `strength` percent; `null` notes quantizes the whole clip. */
export const quantizeNotes = (clip: string, notes: Note[] | null, grid: number, strength: number): Command => ({
  type: 'clip.notes.quantize',
  clip,
  notes,
  grid,
  strength,
})
export const transposeNotes = (clip: string, notes: Note[], semitones: number): Command => ({ type: 'clip.notes.transpose', clip, notes, semitones })
/** Copies of notes shifted by `offset` ticks (null: the span of the selection, so the copies follow it). */
export const duplicateNotes = (clip: string, notes: Note[], offset: number | null): Command => ({ type: 'clip.notes.duplicate', clip, notes, offset })
/** Copies of notes end to end up to the tick `until` (#710), their span rounded up to the `grid` step (null: the exact span). */
export const repeatNotes = (clip: string, notes: Note[], until: number, grid: number | null): Command => ({
  type: 'clip.notes.repeat',
  clip,
  notes,
  until,
  grid,
})
/** A shorter clip cuts what lies past its end (`trim`): the edit is one history entry, so one undo brings the notes back. */
export const setClipLength = (clip: string, length: number): Command => ({ type: 'clip.set_length', clip, length, trim: true })
export const setClipLoop = (clip: string, looping: TickLoop | null): Command => ({ type: 'clip.set_loop', clip, looping })

/** The commands of a clip in the launch grid: each gesture of a cell is one of them. */
export const deleteClip = (clip: string): Command => ({ type: 'clip.delete', clip })
export const renameClip = (clip: string, name: string): Command => ({ type: 'clip.rename', clip, name })
/** A copy of a clip in the cell of `scene` of `track` (null: the clip's own track); with no scene, the next free cell below. */
export const duplicateClip = (clip: string, scene: string | null, track: string | null): Command => ({ type: 'clip.duplicate', clip, slot: scene, track })
/** The clip itself in another cell, of its own track (`track` null) or of another MIDI track. */
export const moveClip = (clip: string, scene: string, track: string | null): Command => ({ type: 'clip.move', clip, slot: scene, track })

/**
 * Plays a note of a track to hear what is edited. It names its track: the live input would also
 * reach every armed track, and an audition belongs to the clip being edited.
 */
export const auditionOn = (track: string, note: number, velocity: number): Command => ({ type: 'live.note_on', track, note, velocity })
export const auditionOff = (track: string, note: number): Command => ({ type: 'live.note_off', track, note })

/** Asks the user for audio files to import into an audio track (AU-1): the page's file dialog, from a click. */
export const chooseAudio = (track: string): Command => ({ type: 'clip.choose_audio', track })
/** Stops the audio import under way (AU-1). */
export const cancelAudioImport = (): Command => ({ type: 'clip.cancel_audio_import' })
/** An agent's audio file from a link (#664): the page asks the user, then downloads and imports it. */
export type ImportAudioUrl = Extract<Command, { type: 'clip.import_audio_url' }>
export const isImportAudioUrl = (command: Command): command is ImportAudioUrl => command.type === 'clip.import_audio_url'
/** The page's answer to an agent's `clip.import_audio_url`: the clip it made and the file's SHA-256. */
export const importAudioUrlOutput = (clip: string | null, media: string) => ({ type: 'clip.import_audio_url', clip, media }) as const
/** An agent's audio file of the project's folder (#778): the helper imports it in Boost, the page makes the clip. */
export type ImportAudioFile = Extract<Command, { type: 'clip.import_audio_file' }>
export const isImportAudioFile = (command: Command): command is ImportAudioFile => command.type === 'clip.import_audio_file'
/** The page's answer to an agent's `clip.import_audio_file`: the clip it made and the file's SHA-256. */
export const importAudioFileOutput = (clip: string | null, media: string) => ({ type: 'clip.import_audio_file', clip, media }) as const
