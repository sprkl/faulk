/**
 * The catalogue commands that have a key of their own (#109): the command palette shows these keys beside them. A command
 * is named here, with the line of the shortcut registry (`shortcuts.ts`) whose keys run it and nothing else; a command that
 * has no line here has no shortcut, or one that does more than this command (Space plays or pauses, Enter depends on the focus).
 * Kept apart from the registry so that the registry says what each key does and this file says which command that is.
 */
import type { MessageKey } from './i18n'

export const SHORTCUT_COMMANDS: Readonly<Record<string, MessageKey>> = {
  'history.undo': 'shortcut.undo',
  'history.redo': 'shortcut.redo',
  'project.save': 'shortcut.save',
  'project.save_as': 'shortcut.saveAs',
  'arrangement.marker.add': 'shortcut.markerAdd',
  'arrangement.duplicate': 'shortcut.clipDuplicate',
  'clip.notes.quantize': 'shortcut.noteQuantize',
}
