/** Transport (`src/views/transport/`): placeholders until the transport epic (TR-4). */

import { defineFeature } from './feature'

export const transport = defineFeature(
  {
    'transport.label': 'Transport',
    'transport.play': 'Play',
    'transport.stop': 'Stop',
    'transport.record': 'Record',
    'transport.loop': 'Loop',
    'transport.position': 'Position',
    'transport.tempo': 'Tempo',
    'transport.signature': 'Time signature',
  },
  {
    'transport.label': 'Transport',
    'transport.play': 'Lecture',
    'transport.stop': 'Stop',
    'transport.record': 'Enregistrer',
    'transport.loop': 'Boucle',
    'transport.position': 'Position',
    'transport.tempo': 'Tempo',
    'transport.signature': 'Signature rythmique',
  },
)
