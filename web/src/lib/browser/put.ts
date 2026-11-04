/**
 * The page's side of a drop (#190): builds the host that `applyItem` reads and runs, puts the item where it goes, selects
 * the track it reached and says what happened in the browser's live region.
 */

import { selectTrack } from '../track-commands'
import { trackLabel } from '../project/tracks'
import type { BrowserItem } from './catalog'
import type { DropStores } from './dnd'
import { applyItem, type DropHost, type DropResult, type DropTarget } from './drop'

/** The host of the page: its dispatcher to run commands, its project store to read the tracks back after each one. */
export const hostOf = ({ project, ui, dispatcher }: DropStores): DropHost => ({
  run: (command) => dispatcher.run(command),
  tracks: () => project.tracks,
  masterDevices: () => project.view?.project.master_devices,
  selectedTrack: () => project.selected?.id ?? null,
  masterSelected: () => ui.masterSelection !== null && ui.masterSelection === (project.selected?.id ?? ''),
})

export function putItem(stores: DropStores, item: BrowserItem, target: DropTarget): DropResult {
  const { project, ui, dispatcher } = stores
  const result = applyItem(hostOf(stores), item, target)
  const entry = result.track ? project.tracks.find((e) => e.id === result.track) : undefined
  // The track the item reached is the selected one: its chain shows in the panel and the computer keyboard plays it.
  if (result.ok && entry && project.selected?.id !== entry.id) dispatcher.run(selectTrack(entry.id))
  const vars: Record<string, string | number> = { device: result.said.device }
  if (entry) vars.track = trackLabel(entry, ui.t)
  if (result.said.position !== undefined) vars.n = result.said.position
  ui.sayBrowser(result.said.key, vars)
  return result
}
