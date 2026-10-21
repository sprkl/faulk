import type { Command } from '../generated/protocol'
import { allNotesOff, controlChange, inputNoteOff, inputNoteOn } from '../synth-commands'

/**
 * Decodes a MIDI message into a live input command, like `decode` in crates/helper/src/midi.rs:
 * channels are merged, note on with velocity 0 is a note off, CC 120/123 release all, any other
 * CC is a control change. The command layer decides which tracks a note reaches.
 */
export function decodeMidi(message: ArrayLike<number>): Command | null {
  if (message.length === 0) return null
  const kind = message[0] & 0xf0
  const byte = (i: number) => (i < message.length ? message[i] & 0x7f : null)
  if (kind === 0x90) {
    const note = byte(1)
    const velocity = byte(2)
    if (note === null || velocity === null) return null
    return velocity === 0 ? inputNoteOff(note) : inputNoteOn(note, velocity)
  }
  if (kind === 0x80) {
    const note = byte(1)
    return note === null ? null : inputNoteOff(note)
  }
  if (kind === 0xb0) {
    const control = byte(1)
    const value = byte(2)
    if (control === null) return null
    if (control === 120 || control === 123) return allNotesOff()
    return value === null ? null : controlChange(control, value)
  }
  return null
}
