import type { Note } from '../../lib/generated/protocol'

/**
 * What copy and cut put aside in the piano roll (PN-2): notes with starts relative to the earliest one. It is the page's,
 * not the system clipboard, and it outlives the clip being edited so that notes can be pasted into another clip.
 */
let board: readonly Note[] = []

export const boardNotes = (): readonly Note[] => board
export function setBoard(notes: readonly Note[]) {
  board = notes
}
