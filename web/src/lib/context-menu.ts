/**
 * The right click in the app (#524). One rule: the browser's own menu never opens, except in a text field. A target that
 * has actions opens the shared menu (`ContextMenu.svelte`) through `UiStore.openMenu`; anywhere else the click does
 * nothing. This file holds the pure parts: what the page does with a `contextmenu` event, the keys that ask for the menu,
 * and where the menu sits.
 */
import { isTextTarget } from './keyboard'

/** One entry of a context menu. Its `run` sends a command (or does what a shortcut of the same view already does). */
export interface MenuItem {
  id: string
  label: string
  /** The shortcut, as written for the person ("Ctrl+D"); shown at the right when the entry has one. */
  shortcut?: string
  /** A second text at the right, in the mono face (the value a reset gives back). Wins over the shortcut. */
  detail?: string
  disabled?: boolean
  /** A colour shown as a dot before the label (the track's hues). */
  swatch?: string
  /** A line above, to group the entries. */
  separator?: boolean
  /** The test id; `ctx-<id>` when missing. */
  testid?: string
  /** The menu stays open (and keeps the focus on the entry) after the pick: an action meant to be repeated, like a tap (#878). */
  stayOpen?: boolean
  run: () => void
}

/** What a menu is: where it opens and what it holds. */
export interface OpenMenu {
  x: number
  y: number
  items: readonly MenuItem[]
  /** The accessible name of the menu. */
  label?: string
}

/** Whether the browser's menu stays: a field where the user types text (copy, paste, spelling), not a list box. */
export function keepsBrowserMenu(target: EventTarget | null): boolean {
  const element = target as Element | null
  if (!element || typeof element.closest !== 'function') return false
  if (element.closest('select')) return false
  return isTextTarget(target)
}

/**
 * What the page does with a `contextmenu` event: nothing when a component handled it, nothing in a text field (the
 * browser's menu opens), else the event is cancelled so that the browser's menu never shows.
 */
export function unhandledContextMenu(event: Pick<Event, 'target' | 'defaultPrevented'>): 'leave' | 'cancel' | 'handled' {
  if (event.defaultPrevented) return 'handled'
  return keepsBrowserMenu(event.target) ? 'leave' : 'cancel'
}

/** Whether a key press asks for the menu of the focused element: Shift+F10 or the Menu key. */
export function isMenuKey(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'ctrlKey' | 'altKey' | 'metaKey'>): boolean {
  if (event.ctrlKey || event.altKey || event.metaKey) return false
  return event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)
}

/** The point a menu opened from the keyboard sits at: the middle of the focused element's box, where a click on it would land. */
export function menuPointOf(box: { left: number; top: number; width: number; height: number }): { x: number; y: number } {
  return { x: box.left + box.width / 2, y: box.top + box.height / 2 }
}

/** The position of a menu of `width` by `height` asked at (x, y): moved back inside the window when it would leave it. */
export function clampMenu(x: number, y: number, width: number, height: number, viewWidth: number, viewHeight: number): { x: number; y: number } {
  const margin = 4
  return {
    x: Math.max(margin, Math.min(x, viewWidth - width - margin)),
    y: Math.max(margin, Math.min(y, viewHeight - height - margin)),
  }
}

/** The index of the next enabled item from `from` in a direction, wrapping round; -1 when none is enabled. */
export function nextEnabled(items: readonly { disabled?: boolean }[], from: number, step: 1 | -1): number {
  const n = items.length
  for (let i = 1; i <= n; i++) {
    const index = (((from + step * i) % n) + n) % n
    if (!items[index]!.disabled) return index
  }
  return -1
}
