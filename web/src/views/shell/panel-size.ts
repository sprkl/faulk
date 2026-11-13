/**
 * Size of the bottom panel: the heights it may take, per view, and how a stored height is read back.
 * The piano roll's minimum keeps one octave readable: tabs (43) + toolbar (ROLL_TOOLBAR_ROOM, two rows) + ruler (28) + 12 rows of 16 + velocity lane (56). The toolbar wraps by width and by font, so the roll reports
 * the height it takes beyond those two rows (`extra`, #887) and the minimum grows by it.
 */
import type { PanelViewId } from '../registry'

/** Smallest height of the panel, per view (px). */
export const PANEL_MIN: Record<PanelViewId, number> = { clips: 120, devices: 120, history: 120, keyboard: 190, 'piano-roll': 392 }
/** Height of the roll's toolbar that `PANEL_MIN['piano-roll']` has room for (two rows, px); a taller one (a third row, a wider font) raises the minimum by the difference. */
export const ROLL_TOOLBAR_ROOM = 73
/** Height the main view keeps at least (px): the panel never grows past what leaves it. */
export const MAIN_MIN = 120
/** Panel width from which the piano roll shows the devices beside it (px): the roll keeps about 600 and the devices column 300. */
export const SPLIT_MIN_WIDTH = 900
/** Keyboard step of the handle (px), and with Shift. */
export const STEP = 16
export const BIG_STEP = 64
/** localStorage keys (the page's preferences, not the project). */
export const HEIGHTS_KEY = 'faulk.panelHeights'
export const COLLAPSED_KEY = 'faulk.panelCollapsed'

/** Smallest height of the panel for a view; `extra` is the height the roll's toolbar takes beyond its room (it counts for the piano roll only). */
export function panelMin(view: PanelViewId, extra = 0): number {
  return PANEL_MIN[view] + (view === 'piano-roll' ? Math.max(0, Math.ceil(extra)) : 0)
}

/** The largest height the panel may take in a body of this height; never below its minimum. */
export function panelMax(view: PanelViewId, bodyHeight: number, extra = 0): number {
  return Math.max(panelMin(view, extra), bodyHeight - MAIN_MIN)
}

/** A height kept between the view's minimum and the room there is, whole pixels. */
export function clampHeight(view: PanelViewId, height: number, bodyHeight: number, extra = 0): number {
  if (!Number.isFinite(height)) return panelMin(view, extra)
  return Math.round(Math.min(panelMax(view, bodyHeight, extra), Math.max(panelMin(view, extra), height)))
}

/** Heights read from storage: only known views and finite numbers above zero; anything else is dropped. */
export function parseHeights(raw: string | null | undefined): Partial<Record<PanelViewId, number>> {
  const out: Partial<Record<PanelViewId, number>> = {}
  if (!raw) return out
  try {
    const data: unknown = JSON.parse(raw)
    if (typeof data !== 'object' || data === null) return out
    for (const view of Object.keys(PANEL_MIN) as PanelViewId[]) {
      const value = (data as Record<string, unknown>)[view]
      if (typeof value === 'number' && Number.isFinite(value) && value > 0) out[view] = Math.round(value)
    }
  } catch {
    // A corrupt preference is forgotten.
  }
  return out
}
