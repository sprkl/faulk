import { defaultLang, LANG_STORAGE_KEY, type Lang, type MessageKey, translate } from '../i18n/index'
import type { MainViewId, PanelViewId } from '../../views/registry'
import { COLLAPSED_KEY, HEIGHTS_KEY, parseHeights } from '../../views/shell/panel-size'
import { clampClipRow } from '../../views/arrangement/automation/layout'
import { storage } from './storage'
import { readLayout, writeLayout, type Layout } from './layout'

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
   * The placement of that clip the piano roll shows (#494): its origin, its play position and its entry in the clip list.
   * Null for the clip's first placement. The roll ignores it when it no longer places the edited clip.
   */
  editedPlacement = $state<string | null>(null)
  /** Points the piano roll at a clip, and at one of its placements (null: its first one). */
  editClip(clip: string, placement: string | null = null) {
    this.editedClip = clip
    this.editedPlacement = placement
  }
  /**
   * What undo or redo just did, in words (#494), for the live regions of the timeline and the piano roll; `n` counts
   * the messages so that the same words twice are said twice. Set by `announceHistory`.
   */
  historySaid = $state<{ text: string; n: number }>({ text: '', n: 0 })
  announceHistory(text: string) {
    this.historySaid = { text, n: this.historySaid.n + 1 }
  }
  /**
   * Musical typing (#470): whether the bare letters of the computer keyboard play notes ('notes') or are left to
   * the shortcuts ('shortcuts'). View state of the session, not project data; one key toggles it (`TYPING_TOGGLE_CODE`).
   */
  typing = $state<'notes' | 'shortcuts'>('notes')

  /** Height the user gave the bottom panel, per view (px): a preference of the page, kept in `localStorage`, not in the project. Missing: the layout's default. */
  panelHeights = $state<Partial<Record<PanelViewId, number>>>(parseHeights(read(HEIGHTS_KEY)))
  /** The bottom panel is folded to its tabs. */
  panelCollapsed = $state(read(COLLAPSED_KEY) === '1')
  /** The bottom panel takes all the room it may (session state, not kept): one key toggles it. */
  panelMaximized = $state(false)
  /** The mixer folds the panel to its tabs; the user unfolded it for this visit (session state, cleared when the view changes). */
  panelRevealed = $state(false)
  /** Bumped when an action targets the panel: it takes the focus (`BottomPanel` reads it). */
  panelFocus = $state(0)
  /** The engine dialog (audio, helper, MIDI, keyboard) is open. Page state, not project state. */
  engineOpen = $state(false)
  /** The shortcut list is open (#474). Page state, not project state. */
  helpOpen = $state(false)

  /**
   * Where a take of MIDI recording goes (#470, spec § 4.1): the last of the grid and the arrangement the user showed.
   * The grid records into the scene of its last focused cell (`recordScene`, else the first scene), the arrangement
   * at the play position. View state, not project data.
   */
  recordView = $state<'grid' | 'arrangement'>('arrangement')
  /** The scene of the grid cell the user last focused or clicked: the grid's target for recording. */
  recordScene = $state<string | null>(null)

  constructor(view: MainViewId, panel: PanelViewId) {
    this.view = $state(view)
    this.panel = $state(panel)
    if (view === 'grid' || view === 'arrangement') this.recordView = view
  }

  t = (key: MessageKey, vars?: Record<string, string | number>) => translate(this.lang, key, vars)

  setLang(lang: Lang) {
    this.lang = lang
    storage()?.setItem(LANG_STORAGE_KEY, lang)
  }

  show(view: MainViewId) {
    if (view !== this.view) this.panelRevealed = false
    this.view = view
    if (view === 'grid' || view === 'arrangement') this.recordView = view
    write(VIEW_KEY, view)
    this.keepSoon()
  }

  /**
   * Whether the panel is shown as its tabs only: folded by the user, or by the mixer, which needs the whole height
   * (the stored fold is not touched, so leaving the mixer brings the panel back as it was).
   */
  get panelFolded(): boolean {
    return this.view === 'mixer' ? !this.panelRevealed || this.panelCollapsed : this.panelCollapsed
  }

  /** Folds or unfolds the panel (in the mixer: for this visit only, unless the user had folded it). */
  toggleFold() {
    if (this.panelFolded) this.unfoldPanel()
    else if (this.view === 'mixer') this.panelRevealed = false
    else this.setPanelCollapsed(true)
  }

  /** Unfolds the panel, whoever folded it. */
  unfoldPanel() {
    if (this.panelCollapsed) this.setPanelCollapsed(false)
    this.panelRevealed = true
  }

  toggleMaximized() {
    if (!this.panelMaximized) this.unfoldPanel()
    this.panelMaximized = !this.panelMaximized
  }

  /**
   * Each view's zoom and scroll, kept while another view is shown (a view is torn down when it is left).
   * Not reactive and not stored: the session's memory, so the view reads it once when it opens.
   */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- not reactive on purpose: read once when a view opens
  private readonly memory = new Map<string, unknown>()

  /**
   * A view hands its zoom and scroll over when it closes, and while they change (kept with the project's layout).
   * `rev` is the {@link layoutRev} the view opened with: a view closing because another project's layout came
   * (the shell renders it again) must not write its old state into the new one.
   */
  remember<T>(id: string, state: T, rev?: number) {
    if (rev !== undefined && rev !== this.layoutRev) return
    this.memory.set(id, state)
    this.keepSoon()
  }

  // --- each project's layout (#473) -------------------------------------------------------------------
  /** The project whose layout the memory, the lane heights and the view are; `null` before one is known. */
  private project: string | null = null
  private keepTimer: ReturnType<typeof setTimeout> | null = null
  /**
   * Bumped when another project's layout replaces the one shown: the shell renders the main view again, so that
   * it reads its zoom and scroll from the new layout.
   */
  layoutRev = $state(0)

  /**
   * The project now open (its identity): its layout comes back (the view, each view's zoom and scroll, the open
   * lanes, the lane heights), or the defaults when it has none. The same project again changes nothing. `isView`
   * says whether a stored view still exists.
   */
  useProject(project: string | null, isView: (view: string) => view is MainViewId) {
    if (project === this.project) return
    this.keepNow()
    this.project = project
    const layout: Layout | null = project ? readLayout(project) : null
    this.memory.clear()
    for (const [id, state] of Object.entries(layout?.memory ?? {})) this.memory.set(id, state)
    this.laneHeights = { ...(layout?.laneHeights ?? {}) }
    if (layout?.view && isView(layout.view) && layout.view !== this.view) {
      this.panelRevealed = false
      this.view = layout.view
    }
    this.layoutRev++
  }

  /** The project took a new identity and is the same work (a copy saved as another folder): its layout goes with it. */
  carryLayout(project: string) {
    this.project = project
    this.keepNow()
  }

  /** Keeps the layout a moment after the last change: a scroll changes it every frame. */
  private keepSoon() {
    if (this.project === null || this.keepTimer !== null) return
    this.keepTimer = setTimeout(() => this.keepNow(), 400)
  }

  /** Keeps the layout now (the page hides, another project opens). */
  keepNow() {
    if (this.keepTimer !== null) clearTimeout(this.keepTimer)
    this.keepTimer = null
    if (this.project === null) return
    writeLayout(this.project, { view: this.view, memory: Object.fromEntries(this.memory), laneHeights: { ...this.laneHeights } })
  }

  /** Forgets the lane heights of tracks the project no longer has. */
  forgetLaneHeights(tracks: ReadonlySet<string>) {
    const kept = Object.entries(this.laneHeights).filter(([id]) => tracks.has(id))
    if (kept.length === Object.keys(this.laneHeights).length) return
    this.laneHeights = Object.fromEntries(kept)
    this.keepSoon()
  }

  /** What the view left when it closed, or `undefined` on its first opening. */
  recall<T>(id: string): T | undefined {
    return this.memory.get(id) as T | undefined
  }

  /** Shows a panel view: unfolds the bottom panel and moves the focus into it. */
  showPanel(panel: PanelViewId) {
    this.panel = panel
    this.unfoldPanel()
    this.panelFocus++
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
    this.showPanel('devices')
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
    this.panelRevealed = false
    this.view = 'arrangement'
    this.recordView = 'arrangement'
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
   * The height the user gave each track's row in the arrangement (px, by track id), #471. View state, not in the
   * project (an agent has no use for it, and saving it would be a project change): kept with the project's layout in
   * `localStorage` (#473, keyed by the project, since track ids belong to one project). Missing: the `lane` token's height.
   */
  laneHeights = $state<Readonly<Record<string, number>>>({})

  setLaneHeight(track: string, height: number | null) {
    const next = { ...this.laneHeights }
    if (height === null) delete next[track]
    else next[track] = clampClipRow(height)
    this.laneHeights = next
    this.keepSoon()
  }

  setPanelHeight(panel: PanelViewId, height: number) {
    this.panelMaximized = false
    this.panelHeights = { ...this.panelHeights, [panel]: height }
    write(HEIGHTS_KEY, JSON.stringify(this.panelHeights))
  }

  setPanelCollapsed(collapsed: boolean) {
    this.panelCollapsed = collapsed
    write(COLLAPSED_KEY, collapsed ? '1' : '0')
  }
}
