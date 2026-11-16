/**
 * The catalogue commands that have a key of their own (#109): the command palette shows these keys beside them. A command
 * is named here with the line of the shortcut registry (`shortcuts.ts`) whose keys run it and nothing else, and the line is
 * `global`: the palette opens in every view, so it only shows keys that work wherever it opens. A command whose key needs a
 * focus (duplicate a placement, quantize notes, add a marker) or does more than that command (Space plays or pauses) is not
 * listed, and a test holds the rule.
 */
import type { MessageKey } from './i18n'

export const SHORTCUT_COMMANDS: Readonly<Record<string, MessageKey>> = {
  'history.undo': 'shortcut.undo',
  'history.redo': 'shortcut.redo',
  'project.save': 'shortcut.save',
  'project.save_as': 'shortcut.saveAs',
}
