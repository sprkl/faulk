import { SINGLE_SYNTH, type Command, type SynthParam } from './generated/protocol'

/**
 * The commands that play and set a synth. Without a track or device they name the ones of
 * `SINGLE_SYNTH`, the ids of the one-synth project the page's core starts with (Web MIDI and the
 * helper's copy still play it until UI-3); the Instrument view names the selected track's.
 */
export const noteOn = (note: number, velocity: number, track: string = SINGLE_SYNTH.track): Command => ({
  type: 'live.note_on',
  track,
  note,
  velocity,
})

export const noteOff = (note: number, track: string = SINGLE_SYNTH.track): Command => ({ type: 'live.note_off', track, note })

export const allNotesOff = (): Command => ({ type: 'live.all_notes_off' })

/**
 * The live input: what a MIDI controller (and, from UI-3, the computer keyboard) plays. It names no
 * track: the command layer sends it to the selected track and the armed ones, and sends the note off
 * to the tracks the note on reached, so a track change while a key is held leaves nothing sounding.
 */
export const inputNoteOn = (note: number, velocity: number): Command => ({ type: 'live.input_note_on', note, velocity })

export const inputNoteOff = (note: number): Command => ({ type: 'live.input_note_off', note })

/** A control change (CC) on the live input, kept for the automation (AM-5). */
export const controlChange = (controller: number, value: number): Command => ({ type: 'live.control_change', controller, value })

export const setParam = (param: SynthParam, value: number, device: string = SINGLE_SYNTH.device): Command => ({
  type: 'device.set_param',
  device,
  param,
  value,
})

export const listMidiInputs = (): Command => ({ type: 'midi.list_inputs' })
