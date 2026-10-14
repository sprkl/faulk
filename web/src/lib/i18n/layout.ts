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
    'tracks.title': 'Tracks',
    'tracks.empty': 'No tracks yet',
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
    'tracks.title': 'Pistes',
    'tracks.empty': 'Aucune piste pour l’instant',
    'panel.label': 'Panneau du bas',
  },
)
