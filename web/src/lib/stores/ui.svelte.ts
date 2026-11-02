import { defaultLang, LANG_STORAGE_KEY, type Lang, type MessageKey, translate } from '../i18n/index'
import type { MainViewId, PanelViewId } from '../../views/registry'
import { COLLAPSED_KEY, HEIGHTS_KEY, parseHeights } from '../../views/shell/panel-size'
import { clampClipRow } from '../../views/arrangement/automation/layout'
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

/** localStorage key of the main view shown last (the page's preference, not the project). */
export const VIEW_KEY = 'faulk.view'

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
    write(VIEW_KEY, view)
  }

  /**
   * Each view's zoom and scroll, kept while another view is shown (a view is torn down when it is left).
   * Not reactive and not stored: the session's memory, so the view reads it once when it opens.
   */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- not reactive on purpose: read once when a view opens
  private readonly memory = new Map<string, unknown>()

  /** A view hands its zoom and scroll over when it closes. */
  remember<T>(id: string, state: T) {
    this.memory.set(id, state)
  }

  /** What the view left when it closed, or `undefined` on its first opening. */
  recall<T>(id: string): T | undefined {
    return this.memory.get(id) as T | undefined
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

  /**
   * The height the user gave each track's row in the arrangement (px, by track id), #471. View state of the session:
   * neither in the project (an agent has no use for it, and saving it would be a project change) nor in `localStorage`
   * (track ids belong to one project). Missing: the `lane` token's height.
   */
  laneHeights = $state<Readonly<Record<string, number>>>({})

  setLaneHeight(track: string, height: number | null) {
    const next = { ...this.laneHeights }
    if (height === null) delete next[track]
    else next[track] = clampClipRow(height)
    this.laneHeights = next
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
