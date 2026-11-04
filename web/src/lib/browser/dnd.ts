/**
 * The drag and drop of the device browser (#190): what a drag carries, the drop targets that accept it, and where in a
 * chain a drop falls. A drag's data cannot be read before the drop, so the item being dragged is kept in `ui.dragged`
 * while it moves; the browser sets it, a drop clears it. Nothing here runs a command: a target calls `putItem`
 * (`put.ts`), which does.
 */

import type { Stores } from '../stores'
import type { UiStore } from '../stores/ui.svelte'
import type { BrowserItem } from './catalog'

/** The type a drag of the browser carries (the page's own: other pages and the desktop send none). */
export const DRAG_TYPE = 'application/x-faulk-browser-item'

/** Starts the drag of an item: the cursor says "copy" (the device stays in the list), and the targets can read `ui.dragged`. */
export function startDrag(ui: UiStore, event: DragEvent, item: BrowserItem) {
  ui.dragged = item
  const transfer = event.dataTransfer
  if (!transfer) return
  transfer.effectAllowed = 'copy'
  transfer.setData(DRAG_TYPE, JSON.stringify({ kind: item.kind, preset: item.preset?.ref ?? null }))
  transfer.setData('text/plain', item.preset ? `${item.name}, ${item.preset.name}` : item.name)
}

export const endDrag = (ui: UiStore) => {
  ui.dragged = null
}

/** Whether a drag carries files from the desktop (an audio file onto the track list: not handled yet, see `audio-drop.ts`). */
export const dragsFiles = (event: DragEvent): boolean => Array.from(event.dataTransfer?.types ?? []).includes('Files')

export interface DropOptions {
  ui: UiStore
  /** Whether the item may be dropped here; a target that does not accept it shows nothing and refuses the drop. */
  accepts: (item: BrowserItem) => boolean
  /** Told when an accepted drag enters or leaves, to style the target. */
  over?: (over: boolean, event: DragEvent) => void
  /** Told on every move over the target (the chain finds its insertion index there). */
  move?: (event: DragEvent) => void
  /** The drop of an accepted item. */
  drop: (item: BrowserItem, event: DragEvent) => void
  /** Files dropped from the desktop: when the target takes them. */
  files?: (files: File[], event: DragEvent) => void
}

/**
 * Makes an element a drop target for the browser's items (and, with `files`, for files). It is a Svelte action:
 * `use:dropTarget={options}`. `data-drop-over` is set while an accepted drag is over it.
 */
export function dropTarget(node: HTMLElement, initial: DropOptions) {
  let options = initial
  let inside = 0

  const wants = (event: DragEvent) => (options.ui.dragged !== null && options.accepts(options.ui.dragged)) || (options.files !== undefined && dragsFiles(event))
  const setOver = (over: boolean, event: DragEvent) => {
    if (over) node.setAttribute('data-drop-over', '')
    else node.removeAttribute('data-drop-over')
    options.over?.(over, event)
  }

  function onEnter(event: DragEvent) {
    if (!wants(event)) return
    inside++
    if (inside === 1) setOver(true, event)
  }
  function onOver(event: DragEvent) {
    if (!wants(event)) return
    // Taking the event is what makes the element a drop target.
    event.preventDefault()
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy'
    options.move?.(event)
  }
  function onLeave(event: DragEvent) {
    if (inside === 0) return
    inside--
    if (inside === 0) setOver(false, event)
  }
  function onDrop(event: DragEvent) {
    inside = 0
    setOver(false, event)
    // A target inside this one (a lane in the track list) already took the drop: it acts once.
    if (event.defaultPrevented) return
    const item = options.ui.dragged
    const accepted = item !== null && options.accepts(item)
    const files = !accepted && options.files !== undefined && dragsFiles(event) ? Array.from(event.dataTransfer?.files ?? []) : null
    if (!accepted && files === null) return
    event.preventDefault()
    options.ui.dragged = null
    if (accepted) options.drop(item, event)
    else options.files?.(files!, event)
  }

  node.addEventListener('dragenter', onEnter)
  node.addEventListener('dragover', onOver)
  node.addEventListener('dragleave', onLeave)
  node.addEventListener('drop', onDrop)
  return {
    update(next: DropOptions) {
      options = next
    },
    destroy() {
      node.removeEventListener('dragenter', onEnter)
      node.removeEventListener('dragover', onOver)
      node.removeEventListener('dragleave', onLeave)
      node.removeEventListener('drop', onDrop)
      node.removeAttribute('data-drop-over')
    },
  }
}

/** The box of an effect's card in a chain, with its place among the effects. */
export interface CardBox {
  index: number
  left: number
  top: number
  right: number
  bottom: number
}

/**
 * Where among the effects a drop at (`x`, `y`) falls: before the first card that is below the pointer, or on its
 * row and right of the pointer's middle line; the end of the chain when the pointer is past every card. The cards
 * wrap onto rows, so the rows are read top to bottom, each left to right (the order the cards are given in).
 */
export function insertionIndex(cards: readonly CardBox[], x: number, y: number): number {
  for (const card of cards) {
    if (y < card.top) return card.index
    if (y < card.bottom && x < (card.left + card.right) / 2) return card.index
  }
  return cards.length === 0 ? 0 : cards[cards.length - 1]!.index + 1
}

/** The two kinds of target, for the stores a page component has at hand. */
export type DropStores = Pick<Stores, 'project' | 'ui' | 'dispatcher'>
