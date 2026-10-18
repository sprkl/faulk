/** The DAW's layout (`src/views/shell/`): view names, track list, bottom panel. */

import { defineFeature } from './feature'

export const layout = defineFeature(
  {
    'views.nav': 'Views',
    'views.arrangement': 'Arrangement',
    'views.devices': 'Devices',
    'views.grid': 'Grid',
    'views.mixer': 'Mixer',
    'views.pianoRoll': 'Piano roll',
    'views.play': 'Instrument',
    'layout.soon': 'Not available yet',
    'play.instrument': 'Instrument',
    'play.noSynth': 'This track’s instrument has no panel yet. Select an Oscillator track to shape its sound.',
    'play.audioTrack': 'An audio track plays clips, not notes. Select a MIDI track to play.',
    'play.noTrack': 'No track selected. Select or add a track to play.',
    'panel.label': 'Bottom panel',
  },
  {
    'views.nav': 'Vues',
    'views.arrangement': 'Arrangement',
    'views.devices': 'Devices',
    'views.grid': 'Grille',
    'views.mixer': 'Mixeur',
    'views.pianoRoll': 'Piano roll',
    'views.play': 'Instrument',
    'layout.soon': 'Pas encore disponible',
    'play.instrument': 'Instrument',
    'play.noSynth': 'L’instrument de cette piste n’a pas encore de panneau. Sélectionne une piste Oscillator pour modeler son son.',
    'play.audioTrack': 'Une piste audio joue des clips, pas des notes. Sélectionne une piste MIDI pour jouer.',
    'play.noTrack': 'Aucune piste sélectionnée. Sélectionne ou ajoute une piste pour jouer.',
    'panel.label': 'Panneau du bas',
  },
)
