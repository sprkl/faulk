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
    'views.keyboard': 'Keyboard',
    'layout.soon': 'Not available yet',
    'panel.label': 'Bottom panel',
    'panel.resize': 'Resize the bottom panel',
    'panel.collapse': 'Collapse the bottom panel',
    'panel.expand': 'Expand the bottom panel',
    'panel.stepArmed': 'Step entry on: letters play',
    'panel.stepArmedMute': 'Step entry on: audio off',
    'panel.stepArmedHint': 'Step entry is on. The letters play notes until a clip opens in the piano roll (Alt+E or Escape to stop)',
    'panel.stepArmedMuteHint':
      'Step entry is on, but the audio is not started: the letters play nothing until it is, and write once a clip opens (Alt+E or Escape to stop)',
    'panel.maximize': 'Maximize the bottom panel (Alt+M)',
    'panel.restore': 'Restore the bottom panel size (Alt+M)',
  },
  {
    'views.nav': 'Vues',
    'views.arrangement': 'Arrangement',
    'views.devices': 'Devices',
    'views.grid': 'Grille',
    'views.mixer': 'Mixeur',
    'views.pianoRoll': 'Piano roll',
    'views.keyboard': 'Clavier',
    'layout.soon': 'Pas encore disponible',
    'panel.label': 'Panneau du bas',
    'panel.resize': 'Redimensionner le panneau du bas',
    'panel.collapse': 'Replier le panneau du bas',
    'panel.expand': 'Déplier le panneau du bas',
    'panel.stepArmed': 'Saisie pas à pas : les lettres jouent',
    'panel.stepArmedMute': 'Saisie pas à pas : audio coupé',
    'panel.stepArmedHint':
      'La saisie pas à pas est activée. Les lettres jouent des notes jusqu’à l’ouverture d’un clip dans le piano roll (Alt+E ou Échap pour l’arrêter)',
    'panel.stepArmedMuteHint':
      'La saisie pas à pas est activée, mais l’audio n’est pas démarré : les lettres ne jouent rien tant qu’il ne l’est pas, et écrivent dès qu’un clip s’ouvre (Alt+E ou Échap pour l’arrêter)',
    'panel.maximize': 'Agrandir le panneau du bas (Alt+M)',
    'panel.restore': 'Rendre sa taille au panneau du bas (Alt+M)',
  },
)
