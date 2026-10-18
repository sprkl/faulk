/** Audio engine start, mode and level meter. */

import { defineFeature } from './feature'

export const engine = defineFeature(
  {
    'start.title': 'Start the audio engine',
    'start.body': 'Browsers only play sound after a click.',
    'start.button': 'Start audio',
    'start.failed': 'The audio engine could not start in this browser.',
    'start.pairPending': 'Once audio starts, Faulk connects to the helper by itself.',
    'engine.web': 'Web',
    'engine.boost': 'Boost',
    'engine.latency': '~{ms} ms (estimated)',
    'engine.latencyUnknown': 'latency unknown',
    'engine.voices': 'Voices: {n}',
    'meter.label': 'Output level',
    'meter.clip': 'CLIP',
    'audio.error': 'The helper’s audio output stopped: back to Web mode.',
    'engine.streamError': 'The audio engine stopped taking changes. Restart audio to continue.',
  },
  {
    'start.title': 'Démarrer le moteur audio',
    'start.body': 'Les navigateurs ne jouent du son qu’après un clic.',
    'start.button': 'Démarrer l’audio',
    'start.failed': 'Le moteur audio n’a pas pu démarrer dans ce navigateur.',
    'start.pairPending': 'Dès que l’audio démarre, Faulk se connecte tout seul au helper.',
    'engine.web': 'Web',
    'engine.boost': 'Boost',
    'engine.latency': '~{ms} ms (estimée)',
    'engine.latencyUnknown': 'latence inconnue',
    'engine.voices': 'Voix : {n}',
    'meter.label': 'Niveau de sortie',
    'meter.clip': 'SATURE',
    'audio.error': 'La sortie audio du helper s’est arrêtée : retour en mode Web.',
    'engine.streamError': 'Le moteur audio n’accepte plus les changements. Relance l’audio pour continuer.',
  },
)
