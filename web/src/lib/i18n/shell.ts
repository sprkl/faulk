/** Page chrome of the DAW (`/app`): home link, language switch, panic. */

import { defineFeature } from './feature'

export const shell = defineFeature(
  {
    'app.home': 'Faulk home page',
    panic: 'All notes off',
    'density.title': 'Row density',
    'density.comfortable': 'Comfortable',
    'density.compact': 'Compact',
    'lang.title': 'Language',
    'lang.en': 'English',
    'lang.fr': 'Français',
    'shell.skipTimeline': 'Skip to the timeline',
    'shell.skipMain': 'Skip to the main view',
  },
  {
    'app.home': 'Page d’accueil de Faulk',
    panic: 'Couper toutes les notes',
    'density.title': 'Densité des lignes',
    'density.comfortable': 'Confort',
    'density.compact': 'Compacte',
    'lang.title': 'Langue',
    'lang.en': 'English',
    'lang.fr': 'Français',
    'shell.skipTimeline': 'Aller à la timeline',
    'shell.skipMain': 'Aller à la vue principale',
  },
)
