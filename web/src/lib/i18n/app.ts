/**
 * Text of the DAW (`/app`) in English and French, assembled from one file per feature. To add a
 * feature: create `<feature>.ts` with `defineFeature`, then add one line to `features` and one to
 * `messages`. Every key must exist in both languages and in one feature only (`i18n.test.ts`).
 */

import { arrangement } from './arrangement'
import { automation } from './automation'
import { browser } from './browser'
import { audio } from './audio'
import { clips } from './clips'
import { audioExport } from './export'
import { engine } from './engine'
import type { FeatureMessages } from './feature'
import { gesture } from './gesture'
import { grid } from './grid'
import { helper } from './helper'
import { history } from './history'
import { controls } from './controls'
import { devices } from './devices'
import { drums } from './drums'
import { compressor } from './compressor'
import { ceiling } from './ceiling'
import { agentConfirm } from './confirm'
import { echo } from './echo'
import { filter } from './filter'
import { stack } from './stack'
import { lattice } from './lattice'
import { scatter } from './scatter'
import { reverb } from './reverb'
import { keys } from './keys'
import { layout } from './layout'
import { fill, type Lang } from './lang'
import { midi } from './midi'
import { mixer } from './mixer'
import { pianoRoll } from './piano-roll'
import { presets } from './presets'
import { project } from './project'
import { refusal } from './refusal'
import { scale } from './scale'
import { shell } from './shell'
import { shortcuts } from './shortcuts'
import { synth } from './synth'
import { tracks } from './tracks'
import { transport } from './transport'
import { trim } from './trim'
import { hiccup } from './hiccup'
import { ripple } from './ripple'

export { DEVICE_NAME_KEYS } from './synth'

/** Every feature of the DAW, in the order of the screen. One line per feature. */
export const features = {
  shell,
  shortcuts,
  layout,
  tracks,
  browser,
  transport,
  scale,
  engine,
  helper,
  midi,
  synth,
  drums,
  echo,
  reverb,
  filter,
  stack,
  lattice,
  scatter,
  compressor,
  ceiling,
  trim,
  hiccup,
  ripple,
  devices,
  presets,
  keys,
  pianoRoll,
  grid,
  gesture,
  project,
  mixer,
  controls,
  arrangement,
  automation,
  clips,
  audio,
  audioExport,
  history,
  refusal,
  agentConfirm,
} as const satisfies Record<string, FeatureMessages>

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
