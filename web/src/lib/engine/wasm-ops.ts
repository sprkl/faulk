import { PARAM_IDS, type Command } from '../generated/protocol'

/** One call into the wasm engine: `[op, a, b]`. */
export type WasmOp =
  | readonly ['note_on', number, number]
  | readonly ['note_off', number, number]
  | readonly ['all_off', number, number]
  | readonly ['set_param', number, number]

/**
 * The wasm call a command maps to, or null for commands the engine does not handle
 * (MIDI device management, done by the page). Parameter ids come from the Rust
 * schema (`PARAM_IDS`), never hard-coded.
 */
export function commandToOp(command: Command): WasmOp | null {
  switch (command.type) {
    case 'note.on':
      return ['note_on', command.note, command.velocity]
    case 'note.off':
      return ['note_off', command.note, 0]
    case 'note.all_off':
      return ['all_off', 0, 0]
    case 'synth.set_param':
      return ['set_param', PARAM_IDS[command.param], command.value]
    case 'midi.list_inputs':
    case 'midi.select_input':
      return null
  }
}
