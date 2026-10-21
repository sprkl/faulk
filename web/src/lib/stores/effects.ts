import type { Command } from '../generated/protocol'

/**
 * What the page itself does for a command the project applied, beside the project and the
 * engine: the Web MIDI input it opens and the notes the piano strip shows as sounding. Neither
 * is in the project or in a `ProjectEvent` (a played note changes nothing in the project; the
 * Web MIDI input is the page's own device, the helper's input is the helper's), so this
 * table names the commands for them. The dispatcher knows none.
 */
export interface EffectPorts {
  /** Which engine plays: the page's input device is only the page's in Web mode. */
  mode: 'web' | 'native'
  /** The page's Web MIDI input, `null` before audio starts or where Web MIDI does not exist. */
  midi: { select(id: string | null): void; refresh(): void } | null
  /** The input chosen in Web mode, reopened when coming back from Boost. */
  rememberWebInput(id: string | null): void
  markNote(note: number, velocity: number): void
  clearNotes(): void
}

export function applyEffects(command: Command, ports: EffectPorts) {
  switch (command.type) {
    case 'midi.select_input':
      if (ports.mode !== 'web') return
      ports.rememberWebInput(command.id ?? null)
      ports.midi?.select(command.id ?? null)
      return
    case 'midi.list_inputs':
      if (ports.mode === 'web') ports.midi?.refresh()
      return
    case 'live.note_on':
    case 'live.input_note_on':
      ports.markNote(command.note, command.velocity)
      return
    case 'live.note_off':
    case 'live.input_note_off':
      ports.markNote(command.note, 0)
      return
    case 'live.all_notes_off':
      ports.clearNotes()
      return
  }
}
