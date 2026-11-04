/**
 * The project's key and scale (#695) on the page: the scales in the order to list them, their pitch classes, and their
 * names in the user's language. The degrees come from the Rust crate (`SCALE_INTERVALS`, `project::ScaleMode`): the page
 * never keeps a table of its own.
 */
import { SCALE_INTERVALS, type Scale, type ScaleKind } from './generated/protocol'
import type { MessageKey } from './i18n/app'

type T = (key: MessageKey, vars?: Record<string, string | number>) => string

export type ScaleMode = Scale['mode']

/** Every scale, in the order the picker lists them. */
export const SCALE_MODES: readonly ScaleMode[] = SCALE_INTERVALS.map((s) => s.mode)

/** The twelve roots, 0 (C) to 11 (B). */
export const ROOTS: readonly number[] = Array.from({ length: 12 }, (_, i) => i)

/** The pitch classes (0 = C … 11 = B) of a scale, rising; `null` for no scale. */
export function pitchClasses(scale: Scale | null | undefined): ReadonlySet<number> | null {
  if (!scale) return null
  const entry = SCALE_INTERVALS.find((s) => s.mode === scale.mode)
  if (!entry) return null
  return new Set(entry.intervals.map((i) => (((scale.root + i) % 12) + 12) % 12))
}

/** What `project.set_scale` names for a project's scale: `chromatic` (root 0) for none. */
export function kindOf(scale: Scale | null | undefined): { root: number; mode: ScaleKind } {
  return scale ? { root: scale.root, mode: scale.mode } : { root: 0, mode: 'chromatic' }
}

const rootName = (root: number, t: T) => t(`scale.root.${((root % 12) + 12) % 12}` as MessageKey)

/** The full name, "F natural minor"; "No scale" for none. */
export function scaleName(scale: Scale | null | undefined, t: T): string {
  if (!scale) return t('scale.none')
  return t(`scale.name.${scale.mode}`, { root: rootName(scale.root, t) })
}

/** The short form the transport bar shows, "Fm", "D dor."; the word "Scale" for none. */
export function scaleShort(scale: Scale | null | undefined, t: T): string {
  if (!scale) return t('scale.noneShort')
  return t(`scale.short.${scale.mode}`, { root: rootName(scale.root, t) })
}

/** A root's name, "F", "Fa". */
export const rootLabel = rootName
