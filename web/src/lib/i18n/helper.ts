/** Native helper: detection, pairing and mode switch. */

import { defineFeature } from './feature'

export const helper = defineFeature(
  {
    'helper.title': 'Native helper',
    'helper.checking': 'Looking for the helper…',
    'helper.absent': 'No helper found. Start faulk-helper for low latency and native MIDI.',
    'helper.helperOlder':
      'This helper is older than the page (protocol {v}). Restart it to install its update, or download it again from get.faulk.app/helper/windows.',
    'helper.helperNewer': 'This page is older than your helper (protocol {v}). Reload the page.',
    'helper.pairPrompt': 'Type the code the helper shows.',
    'helper.code': 'Pairing code',
    'helper.pair': 'Pair',
    'helper.refused': 'Wrong code. {n} attempts left.',
    'helper.refusedNew': 'Wrong or expired code: the helper shows a new one.',
    'helper.failed': 'Pairing failed. Is the helper still running?',
    'helper.useBoost': 'Switch to Boost',
    'helper.useWeb': 'Back to Web',
    'helper.lost': 'Connection to the helper lost: back to Web mode.',
    'helper.replaced': 'Another page took over the helper: back to Web mode.',
    'helper.recheck': 'Check again',
  },
  {
    'helper.title': 'Helper natif',
    'helper.checking': 'Recherche du helper…',
    'helper.absent': 'Aucun helper trouvé. Lance faulk-helper pour une latence basse et le MIDI natif.',
    'helper.helperOlder':
      'Ce helper est plus ancien que la page (protocole {v}). Relance-le pour installer sa mise à jour, ou retélécharge-le sur get.faulk.app/helper/windows.',
    'helper.helperNewer': 'Cette page est plus ancienne que ton helper (protocole {v}). Recharge la page.',
    'helper.pairPrompt': 'Tape le code affiché par le helper.',
    'helper.code': 'Code d’appairage',
    'helper.pair': 'Appairer',
    'helper.refused': 'Code incorrect. Encore {n} essais.',
    'helper.refusedNew': 'Code incorrect ou expiré : le helper en affiche un nouveau.',
    'helper.failed': 'Échec de l’appairage. Le helper tourne-t-il toujours ?',
    'helper.useBoost': 'Passer en Boost',
    'helper.useWeb': 'Revenir en Web',
    'helper.lost': 'Connexion au helper perdue : retour en mode Web.',
    'helper.replaced': 'Une autre page a pris le helper : retour en mode Web.',
    'helper.recheck': 'Chercher à nouveau',
  },
)
