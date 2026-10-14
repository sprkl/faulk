/** Page chrome of the DAW (`/app`): tagline, home link, language switch, panic. */

import { defineFeature } from './feature'

export const shell = defineFeature(
  {
    'app.tagline': 'Open DAW, in your browser',
    'app.home': 'Faulk home page',
    'panic': 'All notes off',
    'lang.switch': 'Français',
  },
  {
    'app.tagline': 'DAW ouvert, dans ton navigateur',
    'app.home': 'Page d’accueil de Faulk',
    'panic': 'Couper toutes les notes',
    'lang.switch': 'English',
  },
)
