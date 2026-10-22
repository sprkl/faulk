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
/** A shorter clip cuts what lies past its end (`trim`): the edit is one history entry, so one undo brings the notes back. */
export const setClipLength = (clip: string, length: number): Command => ({ type: 'clip.set_length', clip, length, trim: true })
export const setClipLoop = (clip: string, looping: TickLoop | null): Command => ({ type: 'clip.set_loop', clip, looping })

/**
 * Plays a note of a track to hear what is edited. It names its track: the live input would also
 * reach every armed track, and an audition belongs to the clip being edited.
 */
export const auditionOn = (track: string, note: number, velocity: number): Command => ({ type: 'live.note_on', track, note, velocity })
export const auditionOff = (track: string, note: number): Command => ({ type: 'live.note_off', track, note })
