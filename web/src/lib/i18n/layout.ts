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
    'panel.label': 'Bottom panel',
    'panel.resize': 'Resize the bottom panel',
    'panel.collapse': 'Collapse the bottom panel',
    'panel.expand': 'Expand the bottom panel',
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
    'panel.label': 'Panneau du bas',
    'panel.resize': 'Redimensionner le panneau du bas',
    'panel.collapse': 'Replier le panneau du bas',
    'panel.expand': 'Déplier le panneau du bas',
  },
)
