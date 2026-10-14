/** One feature's text in both languages, the unit of the DAW's catalogue (see `app.ts`). */

import type { Lang } from './lang'

export type FeatureMessages<K extends string = string> = Record<Lang, Record<K, string>>

/**
 * Declares a feature's text. The English and French objects must have exactly the same keys: the
 * compiler rejects a key present in one language only. Keys are unique across features, which
 * `i18n.test.ts` checks (a spread would silently let one feature override another).
 */
export function defineFeature<const K extends string>(en: Record<K, string>, fr: Record<K, string>): FeatureMessages<K> {
  return { en, fr }
}
