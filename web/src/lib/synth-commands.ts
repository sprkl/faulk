import { SINGLE_SYNTH, type Command, type SynthParam } from './generated/protocol'

/**
 * The commands that play and set the one synth. They name its track and device by the ids of
 * `SINGLE_SYNTH`, the ids of the one-synth project the page's core starts with.
 */
export const noteOn = (note: number, velocity: number): Command => ({
  type: 'live.note_on',
  track: SINGLE_SYNTH.track,
  note,
  velocity,
})

export const noteOff = (note: number): Command => ({ type: 'live.note_off', track: SINGLE_SYNTH.track, note })

export const allNotesOff = (): Command => ({ type: 'live.all_notes_off' })

export const setParam = (param: SynthParam, value: number): Command => ({
  type: 'device.set_param',
  device: SINGLE_SYNTH.device,
  param,
  value,
})

export const listMidiInputs = (): Command => ({ type: 'midi.list_inputs' })
