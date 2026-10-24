/**
 * The behaviour every continuous control shares (knobs, faders, pans, the velocity lane): a drag
 * with a magnet on the default value and a fine mode, and the keyboard. Pure and free of the DOM,
 * so the tuning is tested. A control is a position from 0 to 1; the owner converts it to its own
 * unit and sends the command.
 *
 * - The drag is relative to the value the control had when it was grabbed: pressing does not move
 *   it, so a double click (reset) never makes an intermediate step.
 * - The magnet is a plateau: the value stays on the default while the pointer travels `MAGNET_PX`
 *   on either side of it (less on a short control), then moves on from the default. It uses up
 *   pointer travel instead of skipping values, so every value stays reachable.
 * - The fine mode (Ctrl, Cmd on macOS): the pointer travel counts `FINE_RATIO` times less and there
 *   is no magnet. Pressing or releasing the key in the middle of a drag does not make the value jump.
 */

/** The half width of the magnet's plateau, in pixels of pointer travel. */
export const MAGNET_PX = 5
/** The magnet never takes more than this share of a control's travel (a short pan would lose its middle). */
export const MAGNET_MAX_SHARE = 0.04
/** How many times finer the fine mode is. */
export const FINE_RATIO = 10
/** A key press moves the position by this much (50 steps over the whole range). */
export const KEY_STEP = 0.02
/** Page Up and Page Down move by this much. */
export const PAGE_STEP = 0.1
/** The pointer must travel this far before the drag starts: a click or a double click changes nothing. */
export const DRAG_SLOP_PX = 3

/** Float noise: a pointer exactly at the edge of the plateau is in it. */
const EPSILON = 1e-9
const clamp01 = (n: number) => (Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0)

/** True when the event asks for the fine mode: Cmd on macOS (Ctrl + click is a right click there), Ctrl elsewhere. */
export function fineKey(event: Pick<MouseEvent, 'ctrlKey' | 'metaKey'>, mac = onMac()): boolean {
  return mac ? event.metaKey : event.ctrlKey
}

function onMac(): boolean {
  return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
}

export interface Drag {
  /** The position for a pointer travel of `px` pixels since the grab (positive raises the value), `fine` when the fine key is held. */
  move(px: number, fine: boolean): number
}

/**
 * A drag from `position` over `travel` pixels for the whole range. `home` is the default position
 * the magnet holds, or null for none.
 */
export function createDrag({ position, home, travel }: { position: number; home: number | null; travel: number }): Drag {
  const span = Math.max(1, travel)
  // Half the plateau, in position units.
  const zone = home === null ? 0 : Math.min(MAGNET_PX, MAGNET_MAX_SHARE * span) / span
  // The drag works on a "track": the position with the plateau stretched out, so that the pointer's
  // travel maps to it linearly. `fromTrack` applies the plateau, `toTrack` is its inverse.
  const fromTrack = (u: number): number => {
    if (home === null) return u
    const d = u - home
    return Math.abs(d) <= zone + EPSILON ? home : u - Math.sign(d) * zone
  }
  const toTrack = (value: number): number => {
    if (home === null) return value
    const d = value - home
    return d === 0 ? home : value + Math.sign(d) * zone
  }
  const lo = toTrack(0)
  const hi = toTrack(1)
  let anchor = toTrack(clamp01(position))
  let anchorPx = 0
  let wasFine = false
  let current = clamp01(position)
  return {
    move(px, fine) {
      if (!Number.isFinite(px)) return current
      if (fine !== wasFine) {
        // The mode changes: carry on from where the value is, at the new rate.
        anchor = fine ? current : toTrack(current)
        anchorPx = px
        wasFine = fine
      }
      const u = anchor + (px - anchorPx) / span / (fine ? FINE_RATIO : 1)
      // The ends clamp the track itself: a drag past an end comes back as soon as the pointer does.
      const [low, high] = fine ? [0, 1] : [lo, hi]
      if (u >= high || u <= low) {
        // Exactly the end, whatever the float arithmetic of the plateau says.
        anchor += (u >= high ? high : low) - u
        current = u >= high ? 1 : 0
      } else current = clamp01(fine ? u : fromTrack(u))
      return current
    },
  }
}

/** What a key does to a control. */
export type KeyAction = { position: number } | { reset: true } | null

/**
 * The effect of a key on a focused control at `position`: the arrows move one step (fine step with the
 * fine key), Page Up and Page Down ten, Home and End go to the ends, Delete and Backspace to the default
 * value. A coarse step that would cross `home` stops on it.
 */
export function keyAction(key: string, { position, home, fine }: { position: number; home: number | null; fine: boolean }): KeyAction {
  let delta: number
  switch (key) {
    case 'ArrowUp':
    case 'ArrowRight':
      delta = fine ? KEY_STEP / FINE_RATIO : KEY_STEP
      break
    case 'ArrowDown':
    case 'ArrowLeft':
      delta = fine ? -KEY_STEP / FINE_RATIO : -KEY_STEP
      break
    case 'PageUp':
      delta = PAGE_STEP
      break
    case 'PageDown':
      delta = -PAGE_STEP
      break
    case 'Home':
      return { position: 0 }
    case 'End':
      return { position: 1 }
    case 'Delete':
    case 'Backspace':
      return { reset: true }
    default:
      return null
  }
  const next = clamp01(position + delta)
  // Stop on the default value instead of stepping over it (not in fine mode, which exists to go past).
  if (!fine && home !== null && (position - home) * (next - home) < 0) return { position: home }
  return { position: next }
}
