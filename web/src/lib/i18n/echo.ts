/**
 * Labels of the Echo device's parameters and of their values, keyed `echo.<key>` (the keys of its
 * descriptor, `DEVICES` in the generated protocol). The device's name is not here: it comes from its
 * descriptor, one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const echo = defineFeature(
  {
    'echo.mode': 'Mode',
    'echo.free': 'Free',
    'echo.synced': 'Synced',
    'echo.time': 'Time',
    'echo.division': 'Note value',
    'echo.thirty_second': '1/32',
    'echo.sixteenth': '1/16',
    'echo.eighth_triplet': '1/8T',
    'echo.eighth': '1/8',
    'echo.quarter_triplet': '1/4T',
    'echo.eighth_dotted': '1/8.',
    'echo.quarter': '1/4',
    'echo.quarter_dotted': '1/4.',
    'echo.half': '1/2',
    'echo.whole': '1/1',
    'echo.feedback': 'Feedback',
    'echo.mix': 'Mix',
  },
  {
    'echo.mode': 'Mode',
    'echo.free': 'Libre',
    'echo.synced': 'Au tempo',
    'echo.time': 'Durée',
    'echo.division': 'Valeur de note',
    'echo.thirty_second': '1/32',
    'echo.sixteenth': '1/16',
    'echo.eighth_triplet': '1/8T',
    'echo.eighth': '1/8',
    'echo.quarter_triplet': '1/4T',
    'echo.eighth_dotted': '1/8.',
    'echo.quarter': '1/4',
    'echo.quarter_dotted': '1/4.',
    'echo.half': '1/2',
    'echo.whole': '1/1',
    'echo.feedback': 'Retour',
    'echo.mix': 'Mélange',
  },
)
