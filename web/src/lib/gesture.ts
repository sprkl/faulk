import type { Command } from './generated/protocol'
import { beginGroup, endGroup } from './track-commands'

/**
 * A drag of a fader as one history group, so one undo takes the whole gesture back. Groups do
 * not nest and undo is refused while one is open: `begin` and `end` are idempotent, and the owner
 * ends the gesture on release, on `change`, on losing focus and on being destroyed. A release
 * anywhere in the window ends it too: a drag let go outside the fader never reaches the fader.
 */
export function createGesture(run: (command: Command) => void) {
  let open = false
  const end = () => {
    if (!open) return
    open = false
    globalThis.removeEventListener?.('pointerup', end)
    globalThis.removeEventListener?.('pointercancel', end)
    run(endGroup())
  }
  return {
    // An arrow, not a method: the components pass `gesture.begin` as a callback, detached from the object.
    begin: () => {
      if (open) return
      open = true
      globalThis.addEventListener?.('pointerup', end)
      globalThis.addEventListener?.('pointercancel', end)
      run(beginGroup())
    },
    end,
    get open() {
      return open
    },
  }
}
