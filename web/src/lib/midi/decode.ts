import type { Command } from '../generated/protocol'

/**
 * Decodes a MIDI message into a command, like `decode` in crates/helper/src/midi.rs:
 * channels are merged, note on with velocity 0 is a note off, CC 120/123 release all.
 */
export function decodeMidi(message: ArrayLike<number>): Command | null {
  if (message.length === 0) return null
  const kind = message[0] & 0xf0
  const byte = (i: number) => (i < message.length ? message[i] & 0x7f : null)
  if (kind === 0x90) {
    const note = byte(1)
    const velocity = byte(2)
    if (note === null || velocity === null) return null
    return velocity === 0 ? { type: 'note.off', note } : { type: 'note.on', note, velocity }
  }
  if (kind === 0x80) {
    const note = byte(1)
    return note === null ? null : { type: 'note.off', note }
  }
  if (kind === 0xb0) {
    const control = byte(1)
    if (control === 120 || control === 123) return { type: 'note.all_off' }
  }
  return null
}
