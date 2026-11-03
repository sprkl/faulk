/**
 * Text of the landing page (`/`, `/en`, `/fr`) in English and French. Every key must exist in
 * both languages.
 *
 * Honesty rules (issue #5): nothing that is not built is presented as available, and no latency
 * figure appears until it is measured. `landing.test.ts` enforces the latency rule.
 */

import type { Lang } from './lang'

const en = {
  'meta.title': 'Faulk · Open DAW for humans and agents',
  'meta.description': 'Faulk is an open DAW that opens in your browser, where you’ll build your own instruments and effects. Under construction.',
  skip: 'Skip to content',
  'nav.label': 'Main',
  'nav.home': 'Faulk, home',
  'nav.modes': 'Modes',
  'nav.boost': 'Boost',
  'nav.modules': 'Modules',
  'nav.journey': 'Journey',
  'nav.open': 'Open Faulk',
  'lang.label': 'Language',
  'lang.en': 'English',
  'lang.fr': 'Français',

  'hero.metaLeft': 'FLK-0.1 · IN DEVELOPMENT',
  'hero.metaRight': 'Browser + Boost (native low latency)',
  'hero.title': 'OPEN DAW FOR HUMANS AND AGENTS.',
  'hero.lead':
    'Faulk is a DAW that opens in a browser tab, where you’ll build your own instruments and effects. Soon you’ll describe a sound, an AI will write the module and you’ll hear it straight away. When you need studio latency, Boost takes over.',
  'hero.status': 'Faulk is under construction: the first version plays an oscillator from a MIDI keyboard or the computer keyboard.',
  'hero.cta': 'Open Faulk, no install',
  'hero.secondary': 'About Boost',
  'hero.preview': 'FLK-0.1 · PREVIEW COMING SOON',

  'modes.title1': 'THREE MODES.',
  'modes.title2': 'ONE INTERFACE.',
  'modes.caption': 'The three modes of Faulk and where each one stands',
  'modes.colMode': 'MODE',
  'modes.colRuns': 'WHAT RUNS',
  'modes.colLatency': 'LATENCY',
  'modes.colGain': 'WHAT YOU GET',
  'modes.colStatus': 'STATUS',
  'modes.webName': 'WEB',
  'modes.webRuns': 'The audio engine, in your browser',
  'modes.webLatency': 'Estimated by the browser, shown in the app',
  'modes.webGain': 'Zero install, a link is enough',
  'modes.webStatus': 'First version',
  'modes.boostName': 'BOOST',
  'modes.boostRuns': 'A native helper, driven by the same interface',
  'modes.boostLatency': 'Low, to be measured',
  'modes.boostGain': 'Low-latency audio and direct MIDI',
  'modes.boostStatus': 'In development',
  'modes.atelierName': 'ATELIER',
  'modes.atelierRuns': 'The built-in module editor',
  'modes.atelierLatency': 'Same as the active mode',
  'modes.atelierGain': 'Describe, hear and tweak an instrument',
  'modes.atelierStatus': 'Coming',

  'atelier.eyebrow': 'ATELIER · COMING',
  'atelier.title1': 'DESCRIBE.',
  'atelier.title2': 'LISTEN.',
  'atelier.title3': 'TWEAK.',
  'atelier.body':
    'In the Atelier, you’ll write down the sound you hear in your head. Faulk will generate the module’s code, its real knobs and its sound object. Then you turn, compare, keep. The Atelier is in the works.',
  'atelier.example': 'EXAMPLE',
  'atelier.prompt': '“A dry, short kick, a little click on the attack, that doesn’t smear into the bass.”',
  'atelier.module': 'DRY KICK',
  'atelier.pitch': 'PITCH',
  'atelier.pitchValue': '48 Hz',
  'atelier.decay': 'DECAY',
  'atelier.decayValue': '180 ms',
  'atelier.click': 'CLICK',
  'atelier.clickValue': '0.35',
  'atelier.drive': 'DRIVE',
  'atelier.driveValue': '0.40',

  'library.title': 'THE LIBRARY',
  'library.lead': 'Every module will have its sound object, a grainy texture generated from its parameters. The library will open with Faulk’s first modules.',
  'library.browse': 'Browse modules · coming soon',
  'library.sketch': 'Sketch, not available yet',
  'library.instrument': 'Instrument',
  'library.effect': 'Effect',
  'library.haze': 'HAZE',
  'library.crush': 'CRUSH 8',
  'library.echo': 'ECHO',

  'boost.metaLeft': 'FLK-BOOST · IN DEVELOPMENT',
  'boost.metaRight': 'Windows now, macOS soon',
  'boost.title': 'BOOST',
  'boost.body':
    'Same tab, same project: a small native helper takes over the sound, and the indicator turns purple. It brings low-latency audio and direct MIDI.',
  'boost.figure': '— ms',
  'boost.figureCaption': 'Round-trip latency: published once measured',
  'boost.downloads': 'Boost downloads',
  'boost.windows': 'Download for Windows',
  'boost.macos': 'macOS · coming soon',
  'boost.next': 'Run the installer, then choose “Open Faulk” from the tray icon.',

  'footer.label': 'Footer',
  'footer.site': 'faulk.app · in development',
} as const

export type LandingKey = keyof typeof en

const fr: Record<LandingKey, string> = {
  'meta.title': 'Faulk · Le DAW ouvert, pour les humains et les agents',
  'meta.description': 'Faulk est un DAW ouvert qui s’ouvre dans ton navigateur, et où tu fabriqueras tes propres instruments et effets. En construction.',
  skip: 'Aller au contenu',
  'nav.label': 'Principale',
  'nav.home': 'Faulk, accueil',
  'nav.modes': 'Modes',
  'nav.boost': 'Boost',
  'nav.modules': 'Modules',
  'nav.journey': 'Journey',
  'nav.open': 'Ouvrir Faulk',
  'lang.label': 'Langue',
  'lang.en': 'English',
  'lang.fr': 'Français',

  'hero.metaLeft': 'FLK-0.1 · EN DÉVELOPPEMENT',
  'hero.metaRight': 'Navigateur + Boost (natif, faible latence)',
  'hero.title': 'LE DAW OUVERT, POUR LES HUMAINS ET LES AGENTS.',
  'hero.lead':
    'Faulk est un DAW qui s’ouvre dans un onglet, et où tu fabriqueras tes propres instruments et effets. Bientôt, tu décriras un son, une IA écrira le module et tu l’entendras aussitôt. Quand il te faudra la latence d’un studio, Boost prend le relais.',
  'hero.status': 'Faulk est en construction : la première version joue un oscillateur depuis un clavier MIDI ou le clavier de l’ordinateur.',
  'hero.cta': 'Ouvrir Faulk, sans installer',
  'hero.secondary': 'Découvrir Boost',
  'hero.preview': 'FLK-0.1 · APERÇU À VENIR',

  'modes.title1': 'TROIS MODES.',
  'modes.title2': 'UNE INTERFACE.',
  'modes.caption': 'Les trois modes de Faulk et où en est chacun',
  'modes.colMode': 'MODE',
  'modes.colRuns': 'CE QUI TOURNE',
  'modes.colLatency': 'LATENCE',
  'modes.colGain': 'CE QUE TU GAGNES',
  'modes.colStatus': 'ÉTAT',
  'modes.webName': 'WEB',
  'modes.webRuns': 'Le moteur audio, dans ton navigateur',
  'modes.webLatency': 'Estimée par le navigateur, affichée dans l’app',
  'modes.webGain': 'Zéro installation, un lien suffit',
  'modes.webStatus': 'Première version',
  'modes.boostName': 'BOOST',
  'modes.boostRuns': 'Un helper natif, piloté par la même interface',
  'modes.boostLatency': 'Basse, à mesurer',
  'modes.boostGain': 'Audio basse latence et MIDI direct',
  'modes.boostStatus': 'En développement',
  'modes.atelierName': 'ATELIER',
  'modes.atelierRuns': 'L’éditeur de modules intégré',
  'modes.atelierLatency': 'Celle du mode actif',
  'modes.atelierGain': 'Décrire, écouter et ajuster un instrument',
  'modes.atelierStatus': 'À venir',

  'atelier.eyebrow': 'ATELIER · À VENIR',
  'atelier.title1': 'DÉCRIS.',
  'atelier.title2': 'ÉCOUTE.',
  'atelier.title3': 'AJUSTE.',
  'atelier.body':
    'Dans l’Atelier, tu écriras le son que tu entends dans ta tête. Faulk générera le code du module, ses vrais potentiomètres et son objet sonore. Ensuite, tu tournes, tu compares, tu gardes. L’Atelier est en préparation.',
  'atelier.example': 'EXEMPLE',
  'atelier.prompt': '« Un kick sec et court, un peu de clic à l’attaque, qui ne bave pas dans les basses. »',
  'atelier.module': 'DRY KICK',
  'atelier.pitch': 'PITCH',
  'atelier.pitchValue': '48 Hz',
  'atelier.decay': 'DECAY',
  'atelier.decayValue': '180 ms',
  'atelier.click': 'CLIC',
  'atelier.clickValue': '0,35',
  'atelier.drive': 'DRIVE',
  'atelier.driveValue': '0,40',

  'library.title': 'LA BIBLIOTHÈQUE',
  'library.lead':
    'Chaque module aura son objet sonore, une texture granuleuse générée à partir de ses paramètres. La bibliothèque ouvrira avec les premiers modules de Faulk.',
  'library.browse': 'Parcourir les modules · bientôt',
  'library.sketch': 'Esquisse, pas encore disponible',
  'library.instrument': 'Instrument',
  'library.effect': 'Effet',
  'library.haze': 'HAZE',
  'library.crush': 'CRUSH 8',
  'library.echo': 'ECHO',

  'boost.metaLeft': 'FLK-BOOST · EN DÉVELOPPEMENT',
  'boost.metaRight': 'Windows maintenant, macOS bientôt',
  'boost.title': 'BOOST',
  'boost.body':
    'Même onglet, même projet : un petit helper natif prend le son en charge, et l’indicateur passe au violet. Il apporte l’audio basse latence et le MIDI direct.',
  'boost.figure': '— ms',
  'boost.figureCaption': 'Latence aller-retour : publiée une fois mesurée',
  'boost.downloads': 'Téléchargements de Boost',
  'boost.windows': 'Télécharger pour Windows',
  'boost.macos': 'macOS · bientôt',
  'boost.next': 'Lance l’installateur, puis choisis « Open Faulk » dans l’icône de la zone de notification (le menu du helper est en anglais).',

  'footer.label': 'Pied de page',
  'footer.site': 'faulk.app · en développement',
}

export const landingMessages: Record<Lang, Record<LandingKey, string>> = { en, fr }

/** The landing page's text getter for one language. */
export function landingText(lang: Lang): (key: LandingKey) => string {
  return (key) => landingMessages[lang][key]
}

/**
 * Names of devices and modules (the library's sound objects, the Atelier's example module). A device
 * name is one English name, never translated (founder's decision of 2026-10-14): the same text in
 * every language. Parameter labels and descriptions are translated.
 */
export const LANDING_DEVICE_NAME_KEYS: readonly LandingKey[] = ['atelier.module', 'library.haze', 'library.crush', 'library.echo']

/** Keys allowed to contain a duration in milliseconds: module parameter values, not latencies. */
export const NON_LATENCY_MS_KEYS: readonly LandingKey[] = ['atelier.decayValue']
