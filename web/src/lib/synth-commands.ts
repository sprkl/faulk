import { SINGLE_SYNTH, type Command, type SynthParam } from './generated/protocol'

/** Silences every sounding note (the panic button, a lost MIDI input). */
export const allNotesOff = (): Command => ({ type: 'live.all_notes_off' })

/**
 * The live input: what a MIDI controller and the computer keyboard play. It names no
 * track: the command layer sends it to the selected track and the armed ones, and sends the note off
 * to the tracks the note on reached, so a track change while a key is held leaves nothing sounding.
 */
export const inputNoteOn = (note: number, velocity: number): Command => ({ type: 'live.input_note_on', note, velocity })

export const inputNoteOff = (note: number): Command => ({ type: 'live.input_note_off', note })

/** A control change (CC) on the live input, kept for the automation (AM-5). */
export const controlChange = (controller: number, value: number): Command => ({ type: 'live.control_change', controller, value })

/**
 * Sets a device parameter. Without a device it names the synth of `SINGLE_SYNTH`: only the helper's
 * one-track copy relies on that, until BO-1 (#102); the Instrument view names the selected track's device.
 */
export const setParam = (param: SynthParam, value: number, device: string = SINGLE_SYNTH.device): Command => ({
  type: 'device.set_param',
  device,
  param,
  value,
})

export const listMidiInputs = (): Command => ({ type: 'midi.list_inputs' })
