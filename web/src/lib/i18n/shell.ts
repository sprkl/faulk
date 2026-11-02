/** Page chrome of the DAW (`/app`): home link, language switch, panic. */

import { defineFeature } from './feature'

export const shell = defineFeature(
  {
    'app.home': 'Faulk home page',
    panic: 'All notes off',
    'lang.title': 'Language',
    'lang.en': 'English',
    'lang.fr': 'Français',
  },
  {
    'app.home': 'Page d’accueil de Faulk',
    panic: 'Couper toutes les notes',
    'lang.title': 'Langue',
    'lang.en': 'English',
    'lang.fr': 'Français',
  },
)
