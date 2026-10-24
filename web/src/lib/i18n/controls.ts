/** The continuous controls (`src/lib/components/Slider.svelte`): knobs, faders, pans and the velocity lane. */

import { defineFeature } from './feature'

export const controls = defineFeature(
  {
    'controls.default': 'Default value',
    'controls.hint': 'Double-click or Delete to reset. Hold Ctrl (Cmd on macOS) while dragging for fine steps.',
  },
  {
    'controls.default': 'Valeur par défaut',
    'controls.hint': 'Double-clic ou Suppr pour remettre la valeur par défaut. Maintiens Ctrl (Cmd sur macOS) en glissant pour des pas fins.',
  },
)
