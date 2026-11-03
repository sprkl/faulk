/**
 * Each project's layout, kept in the browser (#473): the view shown, each view's zoom and scroll, the open automation
 * lanes and the lane heights come back when the project is opened again (a reload, a reopen). Page state, not project
 * data: an agent has no use for it, and saving it would make every scroll a change to the project's files. Keyed by
 * the project's identity in `localStorage`, at most {@link MAX_PROJECTS} projects (the least recently used goes).
 * A browser that refuses storage just forgets it; a project opened in another browser starts from the defaults.
 */
import { storage } from './storage'

/** What one project's layout holds; every field is optional (an older or partial record). */
export interface Layout {
  /** The main view shown. */
  view?: string
  /** Each view's memory (`UiStore.remember`), by view id. */
  memory?: Record<string, unknown>
  /** The arrangement's row heights, by track id. */
  laneHeights?: Record<string, number>
}

export const LAYOUT_PREFIX = 'faulk.layout.'
/** The recency list of the projects that have a layout, most recent first. */
export const LAYOUT_INDEX = 'faulk.layouts'
export const MAX_PROJECTS = 20

function get(key: string): string | null {
  try {
    return storage()?.getItem(key) ?? null
  } catch {
    return null
  }
}

function set(key: string, value: string) {
  try {
    storage()?.setItem(key, value)
  } catch {
    // Not kept (quota, private window): the layout still follows the session.
  }
}

function remove(key: string) {
  try {
    storage()?.removeItem(key)
  } catch {
    // Nothing to do.
  }
}

function index(): string[] {
  try {
    const parsed: unknown = JSON.parse(get(LAYOUT_INDEX) ?? '[]')
    return Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === 'string') : []
  } catch {
    return []
  }
}

/** The layout kept for a project, `null` when none (or one that does not read). */
export function readLayout(project: string): Layout | null {
  try {
    const parsed: unknown = JSON.parse(get(LAYOUT_PREFIX + project) ?? 'null')
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
    const record = parsed as Record<string, unknown>
    const layout: Layout = {}
    if (typeof record.view === 'string') layout.view = record.view
    if (record.memory && typeof record.memory === 'object' && !Array.isArray(record.memory)) layout.memory = record.memory as Record<string, unknown>
    if (record.laneHeights && typeof record.laneHeights === 'object' && !Array.isArray(record.laneHeights)) {
      layout.laneHeights = Object.fromEntries(
        Object.entries(record.laneHeights as Record<string, unknown>).filter((e): e is [string, number] => typeof e[1] === 'number' && Number.isFinite(e[1])),
      )
    }
    return layout
  } catch {
    return null
  }
}

/** Keeps a project's layout, and drops the least recently used past {@link MAX_PROJECTS}. */
export function writeLayout(project: string, layout: Layout) {
  set(LAYOUT_PREFIX + project, JSON.stringify(layout))
  const order = [project, ...index().filter((id) => id !== project)]
  for (const old of order.slice(MAX_PROJECTS)) remove(LAYOUT_PREFIX + old)
  set(LAYOUT_INDEX, JSON.stringify(order.slice(0, MAX_PROJECTS)))
}
