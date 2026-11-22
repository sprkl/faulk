/** Native helper: detection, pairing and mode switch. */

import { defineFeature } from './feature'

export const helper = defineFeature(
  {
    'helper.title': 'Native helper',
    'helper.checking': 'Looking for the helper…',
    'helper.absent': 'No helper found. Start faulk-helper for low latency and native MIDI.',
    'helper.localAccessPending': 'Your browser may ask to let Faulk reach apps on this device. Allow it so the page can find the helper.',
    'helper.localAccessBlocked':
      'Your browser blocks access to apps on this device, so the page cannot reach the helper. Allow it in the site settings (icon left of the address), then check again.',
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
    'helper.folderNeedsClick': 'The helper’s folder window opens only from a click: click Show the folder… in the Clips panel.',
    'helper.folderNotBoost': 'The helper reads the project’s folder in Boost mode only.',
    'helper.folderInBrowser': 'This project is kept in the browser: save it in a folder first (Save as).',
    'helper.folderNoMarker': 'Faulk cannot write in the project’s folder yet: allow the access, then show the folder to the helper.',
    'helper.recheck': 'Check again',
    'helper.noOutput':
      'The helper has no audio output it can use. Plug in speakers or headphones, or pick another output in your system’s sound settings. Faulk plays in the browser meanwhile and switches to Boost by itself once the output is there.',
    'helper.stayWeb': 'Stay in Web',
  },
  {
    'helper.title': 'Helper natif',
    'helper.checking': 'Recherche du helper…',
    'helper.absent': 'Aucun helper trouvé. Lance faulk-helper pour une latence basse et le MIDI natif.',
    'helper.localAccessPending':
      'Ton navigateur peut te demander d’autoriser Faulk à joindre les applications de cet appareil. Autorise-le pour que la page trouve le helper.',
    'helper.localAccessBlocked':
      'Ton navigateur bloque l’accès aux applications de cet appareil : la page ne peut pas joindre le helper. Autorise-le dans les paramètres du site (icône à gauche de l’adresse), puis clique sur « Chercher à nouveau ».',
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
    'helper.folderNeedsClick': 'La fenêtre de dossier du helper ne s’ouvre que sur un clic : clique sur Montrer le dossier… dans le panneau Clips.',
    'helper.folderNotBoost': 'Le helper ne lit le dossier du projet qu’en mode Boost.',
    'helper.folderInBrowser': 'Ce projet est gardé dans le navigateur : enregistre-le d’abord dans un dossier (Enregistrer sous).',
    'helper.folderNoMarker': 'Faulk ne peut pas encore écrire dans le dossier du projet : autorise l’accès, puis montre le dossier au helper.',
    'helper.recheck': 'Chercher à nouveau',
    'helper.noOutput':
      'Le helper n’a pas de sortie audio utilisable. Branche des enceintes ou un casque, ou choisis une autre sortie dans les réglages son du système. En attendant, Faulk joue dans le navigateur et passe en Boost tout seul dès que la sortie est là.',
    'helper.stayWeb': 'Rester en Web',
  },
)
