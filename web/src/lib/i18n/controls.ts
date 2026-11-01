/** The continuous controls (`src/lib/components/Slider.svelte`): knobs, faders, pans and the velocity lane. */

import { defineFeature } from './feature'

export const controls = defineFeature(
  {
    'controls.default': 'Default value',
    'controls.hint': 'Double-click or Delete to reset. Enter to type a value. Hold Ctrl (Cmd on macOS) while dragging for fine steps.',
    'controls.type': 'Click to type a value',
    'controls.field': 'Value of {name}',
    'controls.valueOf': '{name}, {value}',
    'controls.invalid': 'Value not recognised',
    'controls.limited': 'Limited to {value}',
    'controls.bare': 'A bare number is read in {unit}.',
  },
  {
    'controls.default': 'Valeur par défaut',
    'controls.hint':
      'Double-clic ou Suppr pour remettre la valeur par défaut. Entrée pour saisir une valeur. Maintiens Ctrl (Cmd sur macOS) en glissant pour des pas fins.',
    'controls.type': 'Cliquer pour saisir une valeur',
    'controls.field': 'Valeur de {name}',
    'controls.valueOf': '{name}, {value}',
    'controls.invalid': 'Valeur non reconnue',
    'controls.limited': 'Limité à {value}',
    'controls.bare': 'Un nombre seul = {unit}.',
  },
)
