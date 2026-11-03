import type { Command } from '../../lib/generated/protocol'
import { beginGroup, endGroup } from '../../lib/track-commands'

/**
 * A run of tempo changes that history counts as one entry (taps, wheel notches): the first `apply` opens a history group, every
 * `apply` runs its command inside it, and the group closes `idleMs` after the last `apply` or `touch`, or when `end` is called (a
 * key press or a click elsewhere, destroy). `touch` is for a tap that changes nothing (same tempo): it belongs to the series and
 * keeps it open. Undo is refused while a group is open, so the owner ends the series before anything else happens.
 * `run` returns whether the command was accepted: a refused `begin_group` (another group is open) leaves the series closed and
 * the command is run on its own.
 */
export function createSeries(run: (command: Command) => boolean, idleMs: number) {
  let open = false
  let timer: ReturnType<typeof setTimeout> | undefined
  const end = () => {
    clearTimeout(timer)
    timer = undefined
    if (!open) return
    open = false
    run(endGroup())
  }
  const arm = () => {
    clearTimeout(timer)
    timer = open ? setTimeout(end, idleMs) : undefined
  }
  return {
    apply(command: Command) {
      if (!open) open = run(beginGroup())
      run(command)
      arm()
    },
    touch: arm,
    end,
    get open() {
      return open
    },
  }
}
