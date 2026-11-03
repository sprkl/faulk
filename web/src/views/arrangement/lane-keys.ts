/**
 * The keys of a track's row in the arrangement (#471), as pure functions. On the selected track's row, a key that
 * starts a number (a digit, a sign, a decimal point) opens the volume field with that key typed in it, and Enter or F2
 * open it on the current value. On an unselected row these keys do nothing special: Enter presses the button, which
 * selects the track. None of them is a playing key (`keyboard.ts`): those are letters and Semicolon.
 *
 * The row's height handle (a separator) moves with the arrows, as the bottom panel's handle does.
 */

import { CLIP_ROW_MAX, CLIP_ROW_MIN, clampClipRow } from './automation/layout'

export type VolumeKey = { open: 'value' } | { open: 'typed'; text: string } | null

/** The keys that start a typed volume: a digit, a sign, or a decimal point (either separator). */
const STARTS_NUMBER = /^[0-9.,+-]$/

/** What a key pressed on a track row's head does to the volume field: open it on the value, open it with the key typed, or nothing. */
export function volumeKey(event: Pick<KeyboardEvent, 'key' | 'altKey' | 'ctrlKey' | 'metaKey' | 'repeat' | 'isComposing'>, selected: boolean): VolumeKey {
  if (!selected || event.altKey || event.ctrlKey || event.metaKey || event.isComposing) return null
  if (event.key === 'Enter' || event.key === 'F2') return event.repeat ? null : { open: 'value' }
  if (STARTS_NUMBER.test(event.key)) return { open: 'typed', text: event.key }
  return null
}

/** Pixels per arrow press on the height handle, and with Shift. */
export const HEIGHT_STEP = 4
export const HEIGHT_STEP_BIG = 16

/** The height a key on the row's height handle gives, or `'default'` for Enter (the density's default height, stored as no height), or null for a key it does not use. */
export function heightKey(event: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'altKey' | 'ctrlKey' | 'metaKey'>, height: number): number | 'default' | null {
  if (event.altKey || event.ctrlKey || event.metaKey) return null
  const step = event.shiftKey ? HEIGHT_STEP_BIG : HEIGHT_STEP
  if (event.key === 'ArrowDown') return clampClipRow(height + step)
  if (event.key === 'ArrowUp') return clampClipRow(height - step)
  if (event.key === 'Home') return CLIP_ROW_MIN
  if (event.key === 'End') return CLIP_ROW_MAX
  if (event.key === 'Enter') return 'default'
  return null
}
