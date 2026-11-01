import { defaultLang, LANG_STORAGE_KEY, type Lang, type MessageKey, translate } from '../i18n/index'
import type { MainViewId, PanelViewId } from '../../views/registry'
import { COLLAPSED_KEY, HEIGHTS_KEY, parseHeights } from '../../views/shell/panel-size'
import { storage } from './storage'

/** A preference of the page; a browser that refuses storage (private window, quota) just forgets it. */
function read(key: string): string | null {
  try {
    return storage()?.getItem(key) ?? null
  } catch {
    return null
  }
}
function write(key: string, value: string) {
  try {
    storage()?.setItem(key, value)
  } catch {
    // Not kept; the layout still follows the session.
  }
}

/**
 * State of the page itself, not of the project or the engine: language and the views shown.
 * Switching views is not a command for now; whether the view shown becomes one is CT-4's call
 * (§ 2.4 of the MVP breakdown).
 */
export class UiStore {
  lang = $state<Lang>(defaultLang(storage()?.getItem(LANG_STORAGE_KEY) ?? null, globalThis.navigator?.language))
  /** The registry's view shown in the main area. */
  view: MainViewId
  /** The registry's view shown in the bottom panel. */
  panel: PanelViewId
  /**
   * The clip the piano roll is pointed at (the launch grid sets it): view state, not project data.
   * The roll uses it when it belongs to the selected track and still exists, else its first clip.
   */
  editedClip = $state<string | null>(null)
  /**
   * Musical typing (#470): whether the bare letters of the computer keyboard play notes ('notes') or are left to
   * the shortcuts ('shortcuts'). View state of the session, not project data; one key toggles it (`TYPING_TOGGLE_CODE`).
   */
  typing = $state<'notes' | 'shortcuts'>('notes')

  /** Height the user gave the bottom panel, per view (px): a preference of the page, kept in `localStorage`, not in the project. Missing: the layout's default. */
  panelHeights = $state<Partial<Record<PanelViewId, number>>>(parseHeights(read(HEIGHTS_KEY)))
  /** The bottom panel is folded to its tabs. */
  panelCollapsed = $state(read(COLLAPSED_KEY) === '1')

  constructor(view: MainViewId, panel: PanelViewId) {
    this.view = $state(view)
    this.panel = $state(panel)
  }

  t = (key: MessageKey, vars?: Record<string, string | number>) => translate(this.lang, key, vars)

  setLang(lang: Lang) {
    this.lang = lang
    storage()?.setItem(LANG_STORAGE_KEY, lang)
  }

  show(view: MainViewId) {
    this.view = view
  }

  showPanel(panel: PanelViewId) {
    this.panel = panel
  }

  /**
   * Set when the devices panel shows the master's chain instead of the selected track's: it holds
   * the id of the track selected then (`''` for none), so that any other selection ends it, whether
   * or not the panel is open.
   */
  masterSelection = $state<string | null>(null)

  /** Shows the devices panel on the master's chain (`master`, with the id of the selected track) or on the selected track's. */
  showDevices(master: boolean, selected?: string) {
    this.masterSelection = master ? (selected ?? '') : null
    this.panel = 'devices'
  }

  /**
   * A one-shot request from the mixer or the devices panel to unfold the automation lanes of an owner
   * (a track id, or `'master'`) in the arrangement view. `ArrangementView` consumes it with
   * `takeLanesRequest()` and adds the owner to its `open` set once; it is not a sticky flag, so folding
   * the owner and navigating back does not reopen it. A second request before the first is consumed
   * replaces it.
   */
  openLanes = $state<string | null>(null)

  /** Navigates to the arrangement and requests, once, that the lanes of `owner` be unfolded. */
  showLanes(owner: string) {
    this.openLanes = owner
    this.view = 'arrangement'
  }

  /** The master's lanes (`showLanes` for the master). */
  showMasterLanes() {
    this.showLanes('master')
  }

  /** Reads and clears the pending lanes request: the owner to unfold, or `null` when none was pending. */
  takeLanesRequest(): string | null {
    const owner = this.openLanes
    this.openLanes = null
    return owner
  }

  setPanelHeight(panel: PanelViewId, height: number) {
    this.panelHeights = { ...this.panelHeights, [panel]: height }
    write(HEIGHTS_KEY, JSON.stringify(this.panelHeights))
  }

  setPanelCollapsed(collapsed: boolean) {
    this.panelCollapsed = collapsed
    write(COLLAPSED_KEY, collapsed ? '1' : '0')
  }
}
