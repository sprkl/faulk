import { describe, expect, it } from 'vitest'
import { decodeMidi } from './decode'

// Same vectors as the Rust tests in crates/helper/src/midi.rs.
describe('decodeMidi', () => {
  it('decodes notes on any channel', () => {
    expect(decodeMidi([0x90, 60, 100])).toEqual({ type: 'note.on', note: 60, velocity: 100 })
    expect(decodeMidi([0x9f, 60, 100])).toEqual({ type: 'note.on', note: 60, velocity: 100 })
    expect(decodeMidi([0x90, 60, 0])).toEqual({ type: 'note.off', note: 60 })
    expect(decodeMidi([0x83, 61, 40])).toEqual({ type: 'note.off', note: 61 })
    expect(decodeMidi([0xb0, 123, 0])).toEqual({ type: 'note.all_off' })
    expect(decodeMidi([0xb0, 120, 0])).toEqual({ type: 'note.all_off' })
  })

  it('ignores other and malformed messages', () => {
    expect(decodeMidi([])).toBeNull()
    expect(decodeMidi([0x90])).toBeNull()
    expect(decodeMidi([0x90, 60])).toBeNull()
    expect(decodeMidi([0xb0, 7, 100])).toBeNull()
    expect(decodeMidi([0xf8])).toBeNull()
    expect(decodeMidi([0x90, 0xff, 0xff])).toEqual({ type: 'note.on', note: 127, velocity: 127 })
  })
})
