/**
 * The actions of a track, as the entries of a context menu (#524): the right click on a track's header in the arrangement,
 * and the inspector's actions menu, share the duplicate. Each entry is one command (`track-commands.ts`) through the
 * dispatcher; renaming asks the owner for its field, which then sends `track.rename`.
 */
import type { MenuItem } from './context-menu'
import { chordText } from './shortcuts'
import { copyName, hueKey, TRACK_COLORS, trackColor, trackLabel, type TrackEntry } from './project/tracks'
import type { Stores } from './stores'
import { deleteTrack, duplicateTrack, selectTrack, setTrackColor } from './track-commands'

type TrackStores = Pick<Stores, 'project' | 'ui' | 'dispatcher'>

/** The copy sits right below the source under a free name, and is selected so the keyboard plays it. */
export function duplicateAndSelect({ project, ui, dispatcher }: TrackStores, entry: TrackEntry) {
  const t = ui.t
  const label = trackLabel(entry, t)
  const before = new Set(project.tracks.map((e) => e.id))
  // The names on screen once the copy is in: an unnamed track shows its position, which the copy shifts.
  const after = project.tracks.map((e) => trackLabel({ track: e.track, index: e.index > entry.index ? e.index + 1 : e.index }, t))
  dispatcher.run(duplicateTrack(entry.id, copyName(label, after)))
  const made = project.tracks.find((e) => !before.has(e.id))
  if (made) dispatcher.run(selectTrack(made.id))
}

/**
 * The menu of a track's header: rename, duplicate, delete, and the colour (which opens the sixteen hues in its place, at
 * the same spot). `at` is where the menu opened.
 */
export function trackMenuItems(stores: TrackStores, entry: TrackEntry, at: { x: number; y: number }, rename: () => void): MenuItem[] {
  const { ui, dispatcher } = stores
  const t = ui.t
  const current = trackColor(entry.track.color, entry.index).toLowerCase()
  const hues = (): MenuItem[] =>
    TRACK_COLORS.map((color) => {
      const key = hueKey(color)
      return {
        id: `color-${color.slice(1)}`,
        label: key ? t(key) : color,
        swatch: color,
        detail: color === current ? '✓' : undefined,
        run: () => dispatcher.run(setTrackColor(entry.id, color)),
      }
    })
  return [
    { id: 'rename', label: t('tracks.menu.rename'), shortcut: chordText(['F2']), run: rename },
    { id: 'duplicate', label: t('tracks.menu.duplicate'), run: () => duplicateAndSelect(stores, entry) },
    { id: 'color', label: t('tracks.menu.color'), swatch: current, run: () => ui.openMenuAt(at.x, at.y, hues(), t('tracks.colorPalette')) },
    { id: 'delete', label: t('tracks.menu.delete'), separator: true, run: () => dispatcher.run(deleteTrack(entry.id)) },
  ]
}
