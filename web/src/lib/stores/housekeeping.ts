import type { Command } from '../generated/protocol'
import { allNotesOff, listMidiInputs } from '../synth-commands'

/**
 * The commands the page sends on its own account, not for a user action: release every note
 * before an engine is set aside, ask the helper for its MIDI inputs once it plays. The
 * dispatcher passes them to its own `run` and names none of them.
 */
export const releaseAllNotes = (run: (command: Command) => void) => run(allNotesOff())
export const requestMidiInputs = (run: (command: Command) => void) => run(listMidiInputs())
