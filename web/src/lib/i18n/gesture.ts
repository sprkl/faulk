/** The label that follows the pointer while a clip or a note is moved or resized (#612). */

import { defineFeature } from './feature'

export const gesture = defineFeature(
  {
    'gesture.position': 'Bar {bar}, beat {beat}',
    'gesture.length': 'Length {length}',
    'gesture.bar': '{n} bar',
    'gesture.bars': '{n} bars',
    'gesture.beat': '{n} beat',
    'gesture.beats': '{n} beats',
    'gesture.snap': 'Snap: {name}',
    'gesture.free': 'Free',
    'gesture.fraction': '1/{n} beat',
    'gesture.ctrlFree': '{key}: free',
  },
  {
    'gesture.position': 'Mesure {bar}, temps {beat}',
    'gesture.length': 'Longueur {length}',
    'gesture.bar': '{n} mesure',
    'gesture.bars': '{n} mesures',
    'gesture.beat': '{n} temps',
    'gesture.beats': '{n} temps',
    'gesture.snap': 'Magnétisme : {name}',
    'gesture.free': 'Libre',
    'gesture.fraction': '1/{n} de temps',
    'gesture.ctrlFree': '{key} : libre',
  },
)
