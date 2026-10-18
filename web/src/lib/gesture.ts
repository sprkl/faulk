import type { Command } from './generated/protocol'
import { beginGroup, endGroup } from './track-commands'

/**
 * A drag of a fader as one history group, so one undo takes the whole gesture back. Groups do
 * not nest and undo is refused while one is open: `begin` and `end` are idempotent, and the owner
 * ends the gesture on release, on `change`, on losing focus and on being destroyed.
 */
export function createGesture(run: (command: Command) => void) {
  let open = false
  return {
    begin() {
      if (open) return
      open = true
      run(beginGroup())
    },
    end() {
      if (!open) return
      open = false
      run(endGroup())
    },
    get open() {
      return open
    },
  }
}
