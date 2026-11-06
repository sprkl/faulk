// What the Clips panel says about audio clips in Boost mode (AU-7, #116): the helper plays them from the
// project's folder once it has it (`folder.state`), and says what it needs from the user otherwise.
import type { Command, FolderStatus } from '../generated/protocol'
import type { MessageKey } from '../i18n'

/** `project.choose_helper_folder`: the helper shows its folder window for the project's folder. */
export const chooseHelperFolder: Command = { type: 'project.choose_helper_folder' }

/** A notice of the Clips panel in Boost: its text, and whether it offers to show the folder to the helper. */
export interface BoostFolderNotice {
  key: MessageKey
  vars?: Record<string, string | number>
  /** The "Show the folder…" button goes with it. */
  ask: boolean
}

/**
 * The notice for a project kept in the browser (`inBrowser`), for a folder that refused the page's marker
 * (`linkFailed`: the button tries again), or for the helper's last `folder.state` (`folder`, `null` before
 * its first answer). `null`: nothing to say (the helper reads the folder, or has not answered yet, or the
 * project has no place at all).
 */
export function boostFolderNotice(inBrowser: boolean, folder: { state: FolderStatus } | null, linkFailed = false): BoostFolderNotice | null {
  if (inBrowser) return { key: 'clips.boostBrowser', ask: false }
  if (linkFailed) return { key: 'clips.boostNoMarker', ask: true }
  if (!folder) return null
  const state = folder.state
  switch (state.status) {
    case 'open':
    case 'closed':
      return null
    case 'unknown':
      return { key: 'clips.boostShowFolder', ask: true }
    case 'ambiguous':
      return { key: 'clips.boostAmbiguous', vars: { count: state.count }, ask: true }
    case 'choosing':
      return { key: 'clips.boostChoosing', ask: false }
    case 'refused':
      switch (state.reason) {
        case 'cancelled':
          return { key: 'clips.boostShowFolder', ask: true }
        case 'not_this_folder':
          return { key: 'clips.boostNotThisFolder', ask: true }
        case 'other_project':
          return { key: 'clips.boostOtherProject', ask: true }
        case 'no_project':
          return { key: 'clips.boostNoProject', ask: true }
        case 'unreadable':
          return { key: 'clips.boostUnreadable', ask: true }
        case 'unavailable':
          return { key: 'clips.boostNoDialog', ask: false }
      }
  }
}
