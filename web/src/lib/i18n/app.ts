/**
 * Text of the DAW (`/app`) in English and French, assembled from one file per feature. To add a
 * feature: create `<feature>.ts` with `defineFeature`, then add one line to `features` and one to
 * `messages`. Every key must exist in both languages and in one feature only (`i18n.test.ts`).
 */

import { engine } from './engine'
import type { FeatureMessages } from './feature'
import { grid } from './grid'
import { helper } from './helper'
import { devices } from './devices'
import { drums } from './drums'
import { keys } from './keys'
import { layout } from './layout'
import { fill, type Lang } from './lang'
import { midi } from './midi'
import { pianoRoll } from './piano-roll'
import { shell } from './shell'
import { synth } from './synth'
import { tracks } from './tracks'
import { transport } from './transport'

export { DEVICE_NAME_KEYS } from './synth'

/** Every feature of the DAW, in the order of the screen. One line per feature. */
export const features = { shell, layout, tracks, transport, engine, helper, midi, synth, drums, devices, keys, pianoRoll, grid } as const satisfies Record<string, FeatureMessages>

type Features = typeof features
type UnionToIntersection<U> = (U extends unknown ? (u: U) => void : never) extends (i: infer I) => void ? I : never

export type MessageKey = keyof UnionToIntersection<Features[keyof Features]['en']>

function assemble(lang: Lang): Record<MessageKey, string> {
  return Object.assign({}, ...Object.values(features).map((feature) => feature[lang]))
}

export const messages: Record<Lang, Record<MessageKey, string>> = { en: assemble('en'), fr: assemble('fr') }

export function translate(lang: Lang, key: MessageKey, vars: Record<string, string | number> = {}): string {
  return fill(messages[lang][key], vars)
}
