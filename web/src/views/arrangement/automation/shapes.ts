/**
 * The words of the curves of an automation point (#881), by shape: the name in a list (the "Ramp to…" dialog), the word inside
 * a sentence (a point's cell, what is said), the entry of the point's menu and that of the toolbar's More menu. Every shape has
 * all four, in English and French (`i18n/automation.ts`).
 */

import type { MessageKey } from '../../../lib/i18n'
import type { Shape } from '../../../lib/generated/protocol'

export interface ShapeWords {
  name: MessageKey
  word: MessageKey
  menu: MessageKey
  more: MessageKey
}

export const SHAPE_NAMES: Readonly<Record<Shape, ShapeWords>> = {
  linear: { name: 'automation.nameLinear', word: 'automation.shapeLinear', menu: 'automation.menuLinear', more: 'automation.curveLinear' },
  step: { name: 'automation.nameStep', word: 'automation.shapeStep', menu: 'automation.menuStep', more: 'automation.curveStep' },
  ease_in: { name: 'automation.nameEaseIn', word: 'automation.shapeEaseIn', menu: 'automation.menuEaseIn', more: 'automation.curveEaseIn' },
  ease_out: { name: 'automation.nameEaseOut', word: 'automation.shapeEaseOut', menu: 'automation.menuEaseOut', more: 'automation.curveEaseOut' },
  smooth: { name: 'automation.nameSmooth', word: 'automation.shapeSmooth', menu: 'automation.menuSmooth', more: 'automation.curveSmooth' },
}
