/**
 * The gestures of the interface the command palette lists beside the catalogue's commands (#926): what a musician does with
 * a key or a click and that is no command of its own (a dialog opens, the view zooms, a mode starts). Each names the line of
 * the shortcut registry (`shortcuts.ts`) that holds its keys, so the palette shows them from there, and runs like that
 * shortcut: the dialogs and panels it opens itself, the actions of a view are asked of the view that owns them
 * (`UiStore.requestGesture`), which calls the function its key calls. Every project change they make is a command the view
 * sends through the dispatcher, as for the key.
 */
import type { MessageKey } from './i18n'
import type { UiStore } from './stores/ui.svelte'

export const GESTURE_IDS = [
  'goTo',
  'help',
  'browser',
  'panel',
  'fitSong',
  'clipNew',
  'clipRepeat',
  'clipLengthen',
  'markerAdd',
  'laneRamp',
  'stepEntry',
] as const
export type GestureId = (typeof GESTURE_IDS)[number]

export interface GestureDef {
  id: GestureId
  /** The line of the shortcut registry that holds its keys. */
  shortcut: MessageKey
  /** The view that does it: the shell (it opens a dialog or a panel itself), the arrangement or the piano roll. */
  owner: 'shell' | 'arrangement' | 'piano-roll'
}

export const GESTURES: readonly GestureDef[] = [
  { id: 'goTo', shortcut: 'shortcut.goTo', owner: 'shell' },
  { id: 'help', shortcut: 'shortcut.help', owner: 'shell' },
  { id: 'browser', shortcut: 'shortcut.browser', owner: 'shell' },
  { id: 'panel', shortcut: 'shortcut.panel', owner: 'shell' },
  { id: 'fitSong', shortcut: 'shortcut.fitSong', owner: 'arrangement' },
  { id: 'clipNew', shortcut: 'shortcut.clipNew', owner: 'arrangement' },
  { id: 'clipRepeat', shortcut: 'shortcut.clipRepeat', owner: 'arrangement' },
  { id: 'clipLengthen', shortcut: 'shortcut.clipLengthen', owner: 'arrangement' },
  { id: 'markerAdd', shortcut: 'shortcut.markerAdd', owner: 'arrangement' },
  { id: 'laneRamp', shortcut: 'shortcut.laneRamp', owner: 'arrangement' },
  { id: 'stepEntry', shortcut: 'shortcut.stepEntry', owner: 'piano-roll' },
]

/** The gestures each view takes from `UiStore.gesture`. */
export const ARRANGEMENT_GESTURES: readonly GestureId[] = GESTURES.filter((g) => g.owner === 'arrangement').map((g) => g.id)
export const PIANO_ROLL_GESTURES: readonly GestureId[] = GESTURES.filter((g) => g.owner === 'piano-roll').map((g) => g.id)

/** The page's own state a gesture changes, narrowed to what it needs (a test passes a double). */
export type GestureUi = Pick<UiStore, 'gotoOpen' | 'helpOpen' | 'showBrowser' | 'toggleMaximized' | 'show' | 'showPanel' | 'requestGesture'>

/** Runs a gesture: what its shortcut does, once the palette has closed. */
export function runGesture(id: GestureId, ui: GestureUi): void {
  const def = GESTURES.find((g) => g.id === id)
  if (!def) return
  if (def.owner === 'shell') {
    if (id === 'goTo') ui.gotoOpen = true
    else if (id === 'help') ui.helpOpen = true
    else if (id === 'browser') ui.showBrowser()
    else ui.toggleMaximized()
    return
  }
  if (def.owner === 'arrangement') ui.show('arrangement')
  else ui.showPanel('piano-roll')
  ui.requestGesture(id)
}
