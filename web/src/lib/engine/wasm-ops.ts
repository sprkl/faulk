import { PARAM_IDS, type Command, type SynthParam } from '../generated/protocol'

/** One call into the wasm engine: `[op, a, b]`. */
export type WasmOp =
  | readonly ['note_on', number, number]
  | readonly ['note_off', number, number]
  | readonly ['all_off', number, number]
  | readonly ['set_param', number, number]

/**
 * The wasm call a command maps to, or null for commands the engine does not handle
 * (MIDI device management, done by the page; project edits; a parameter the synth does
 * not have). Every live note and parameter goes to the one synth, whatever the track
 * and device ids. Parameter ids come from the Rust schema (`PARAM_IDS`), never
 * hard-coded.
 */
export function commandToOp(command: Command): WasmOp | null {
  switch (command.type) {
    case 'live.note_on':
      return ['note_on', command.note, command.velocity]
    case 'live.note_off':
      return ['note_off', command.note, 0]
    case 'live.all_notes_off':
      return ['all_off', 0, 0]
    case 'device.set_param':
      return Object.hasOwn(PARAM_IDS, command.param)
        ? ['set_param', PARAM_IDS[command.param as SynthParam], command.value]
        : null
    default:
      return null
  }
}
