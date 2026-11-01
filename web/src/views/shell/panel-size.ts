/**
 * Size of the bottom panel: the heights it may take, per view, and how a stored height is read back.
 * The piano roll's minimum keeps one octave readable: tabs (41) + toolbar (36) + ruler (28) + 12 rows of 16 + velocity lane (56).
 */
import type { PanelViewId } from '../registry'

/** Smallest height of the panel, per view (px). */
export const PANEL_MIN: Record<PanelViewId, number> = { devices: 120, history: 120, 'piano-roll': 360 }
/** Height the main view keeps at least (px): the panel never grows past what leaves it. */
export const MAIN_MIN = 120
/** Keyboard step of the handle (px), and with Shift. */
export const STEP = 16
export const BIG_STEP = 64
/** localStorage keys (the page's preferences, not the project). */
export const HEIGHTS_KEY = 'faulk.panelHeights'
export const COLLAPSED_KEY = 'faulk.panelCollapsed'

/** The largest height the panel may take in a body of this height; never below its minimum. */
export function panelMax(view: PanelViewId, bodyHeight: number): number {
  return Math.max(PANEL_MIN[view], bodyHeight - MAIN_MIN)
}

/** A height kept between the view's minimum and the room there is, whole pixels. */
export function clampHeight(view: PanelViewId, height: number, bodyHeight: number): number {
  if (!Number.isFinite(height)) return PANEL_MIN[view]
  return Math.round(Math.min(panelMax(view, bodyHeight), Math.max(PANEL_MIN[view], height)))
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
