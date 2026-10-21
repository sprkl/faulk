import { describe, expect, it } from 'vitest'
import { decodeMidi } from './decode'
import { allNotesOff, controlChange, inputNoteOff, inputNoteOn } from '../synth-commands'

// Same vectors as the Rust tests in crates/helper/src/midi.rs.
describe('decodeMidi', () => {
  it('decodes notes on any channel', () => {
    expect(decodeMidi([0x90, 60, 100])).toEqual(inputNoteOn(60, 100))
    expect(decodeMidi([0x9f, 60, 100])).toEqual(inputNoteOn(60, 100))
    expect(decodeMidi([0x90, 60, 0])).toEqual(inputNoteOff(60))
    expect(decodeMidi([0x83, 61, 40])).toEqual(inputNoteOff(61))
    expect(decodeMidi([0xb0, 123, 0])).toEqual(allNotesOff())
    expect(decodeMidi([0xb0, 120, 0])).toEqual(allNotesOff())
  })

  it('decodes control changes on any channel', () => {
    expect(decodeMidi([0xb0, 7, 100])).toEqual(controlChange(7, 100))
    expect(decodeMidi([0xb5, 1, 0xff])).toEqual(controlChange(1, 127))
    expect(decodeMidi([0xb0, 7])).toBeNull()
    expect(decodeMidi([0xb0])).toBeNull()
  })

  it('ignores other and malformed messages', () => {
    expect(decodeMidi([])).toBeNull()
    expect(decodeMidi([0x90])).toBeNull()
    expect(decodeMidi([0x90, 60])).toBeNull()
    expect(decodeMidi([0xf8])).toBeNull()
    expect(decodeMidi([0x90, 0xff, 0xff])).toEqual(inputNoteOn(127, 127))
  })
})
