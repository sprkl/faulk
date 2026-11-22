/**
 * Labels of the Scatter device's parameters and of their values, keyed `scatter.<key>` (the keys
 * of its descriptor, `DEVICES` in the generated protocol; #691). The device's name is not here: it
 * comes from its descriptor, one English name never translated (founder's decision of 2026-11-06).
 */

import { defineFeature } from './feature'

export const scatter = defineFeature(
  {
    'scatter.chance': 'Chance',
    'scatter.choices': 'Choices',
    'scatter.interval': 'Interval',
    'scatter.direction': 'Direction',
    'scatter.up': 'Up',
    'scatter.down': 'Down',
    'scatter.around': 'Around',
    'scatter.mode': 'Steps',
    'scatter.semitones': 'Semitones',
    'scatter.scale': 'In scale',
    'scatter.order': 'Order',
    'scatter.random': 'Random',
    'scatter.cycle': 'In turn',
    'scatter.seed': 'Seed',
  },
  {
    'scatter.chance': 'Chance',
    'scatter.choices': 'Choix',
    'scatter.interval': 'Intervalle',
    'scatter.direction': 'Sens',
    'scatter.up': 'Au-dessus',
    'scatter.down': 'Au-dessous',
    'scatter.around': 'Autour',
    'scatter.mode': 'Pas',
    'scatter.semitones': 'Demi-tons',
    'scatter.scale': 'Dans la gamme',
    'scatter.order': 'Ordre',
    'scatter.random': 'Aléatoire',
    'scatter.cycle': 'Tour à tour',
    'scatter.seed': 'Graine',
  },
)
