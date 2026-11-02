/**
 * Text of the Journey page (`/journey`, `/en/journey`, `/fr/journey`) in English and French. Every
 * key must exist in both languages. The entries' own texts live in their files
 * (`lib/journey/entries/`), in both languages too.
 */

import type { Lang } from './lang'

const en = {
  'meta.title': 'Journey · Faulk',
  'meta.description':
    'One track a day, made by Faulk’s product-owner agent in the latest development build: the audio, the production sheet, how far it got and what was missing.',

  'hero.metaLeft': 'JOURNEY · ONE TRACK A DAY',
  'hero.metaRight': 'Made by an agent, in Faulk’s development build',
  'hero.title': 'A TRACK A DAY, MADE BY AN AGENT.',
  'hero.lead':
    'Every day, Faulk’s product-owner agent takes the latest development build, writes a production sheet and builds the track in Faulk, through the interface and through commands. Each day below says how far it got and what Faulk was still missing.',
  'hero.note': 'The audio is Faulk’s own export, encoded to MP3, with no mastering after it.',
  'index.label': 'Latest days',
  'index.count': '{count} days',
  'index.one': '1 day',
  'index.archive': 'Every month',

  'latest.label': 'Latest',
  'earlier.title': 'EARLIER DAYS',
  'archive.title': 'BY MONTH',

  'meta.month': '{month} · Journey · Faulk',
  'meta.monthDescription': 'The tracks Faulk’s product-owner agent made in {month}, day by day.',
  'meta.day': '{title} · Journey · Faulk',
  'month.nav': 'Other months',
  'month.older': 'Earlier: {month}',
  'month.newer': 'Later: {month}',
  'back.home': 'All of Journey',
  'day.nav': 'Other days',
  'day.month': 'All of {month}',
  'day.older': 'Day before: {title}',
  'day.newer': 'Day after: {title}',

  'entry.brief': 'Brief',
  'entry.built': 'How far it got',
  'entry.tested': 'New in Faulk, put to the test',
  'entry.gaps': 'What was missing',
  'entry.form': 'Form',
  'entry.bpm': '{bpm} BPM',
  'entry.bars': '{bars} bars',
  'entry.commit': 'develop {commit}',
  'entry.agent': 'Product-owner agent · {model} · skill {skills}',
  'entry.sheet': 'Production sheet',
  'entry.midi': 'MIDI file',
  'entry.journal': 'Journal entry, #362',
  'entry.cover': 'Cover generated from the track: {tracks} tracks, {bpm} BPM, {key}, {bars} bars.',
  'entry.specs': 'About this track',

  'player.label': 'Player: {title}',
  'player.play': 'Play {title}',
  'player.pause': 'Pause {title}',
  'player.seek': 'Position in {title}',
  'player.valuetext': '{elapsed} of {total}',
  'player.error': 'The audio did not load. The MP3 is also on the CDN:',
  'player.download': 'open the MP3',

  'licence.lead': 'The tracks, MIDI files and production sheets are licensed',
  'licence.name': 'CC BY 4.0',
  'licence.credit': 'Credit: Faulk Journey (sprkl).',
}

export type JourneyKey = keyof typeof en

const fr: Record<JourneyKey, string> = {
  'meta.title': 'Journey · Faulk',
  'meta.description':
    'Un morceau par jour, fait par l’agent product owner de Faulk dans la dernière version de développement : l’audio, la fiche de production, jusqu’où il est allé et ce qui manquait.',

  'hero.metaLeft': 'JOURNEY · UN MORCEAU PAR JOUR',
  'hero.metaRight': 'Fait par un agent, dans la version de développement de Faulk',
  'hero.title': 'UN MORCEAU PAR JOUR, FAIT PAR UN AGENT.',
  'hero.lead':
    'Chaque jour, l’agent product owner de Faulk prend la dernière version de développement, écrit une fiche de production et construit le morceau dans Faulk, par l’interface et par les commandes. Chaque jour ci-dessous dit jusqu’où il est allé et ce qui manquait encore à Faulk.',
  'hero.note': 'L’audio est l’export de Faulk lui-même, encodé en MP3, sans mastering après.',
  'index.label': 'Derniers jours',
  'index.count': '{count} jours',
  'index.one': '1 jour',
  'index.archive': 'Tous les mois',

  'latest.label': 'Le plus récent',
  'earlier.title': 'LES JOURS D’AVANT',
  'archive.title': 'PAR MOIS',

  'meta.month': '{month} · Journey · Faulk',
  'meta.monthDescription': 'Les morceaux faits par l’agent product owner de Faulk en {month}, jour après jour.',
  'meta.day': '{title} · Journey · Faulk',
  'month.nav': 'Autres mois',
  'month.older': 'Avant : {month}',
  'month.newer': 'Après : {month}',
  'back.home': 'Tout Journey',
  'day.nav': 'Autres jours',
  'day.month': 'Tout {month}',
  'day.older': 'Jour d’avant : {title}',
  'day.newer': 'Jour d’après : {title}',

  'entry.brief': 'Brief',
  'entry.built': 'Jusqu’où il est allé',
  'entry.tested': 'Nouveautés de Faulk mises à l’épreuve',
  'entry.gaps': 'Ce qui manquait',
  'entry.form': 'Forme',
  'entry.bpm': '{bpm} BPM',
  'entry.bars': '{bars} mesures',
  'entry.commit': 'develop {commit}',
  'entry.agent': 'Agent product owner · {model} · skill {skills}',
  'entry.sheet': 'Fiche de production',
  'entry.midi': 'Fichier MIDI',
  'entry.journal': 'Entrée du journal, #362',
  'entry.cover': 'Pochette générée à partir du morceau : {tracks} pistes, {bpm} BPM, {key}, {bars} mesures.',
  'entry.specs': 'Sur ce morceau',

  'player.label': 'Lecteur : {title}',
  'player.play': 'Lire {title}',
  'player.pause': 'Mettre en pause {title}',
  'player.seek': 'Position dans {title}',
  'player.valuetext': '{elapsed} sur {total}',
  'player.error': 'L’audio ne s’est pas chargé. Le MP3 est aussi sur le CDN :',
  'player.download': 'ouvrir le MP3',

  'licence.lead': 'Les morceaux, les fichiers MIDI et les fiches de production sont sous licence',
  'licence.name': 'CC BY 4.0',
  'licence.credit': 'Crédit : Faulk Journey (sprkl).',
}

export const journeyMessages: Record<Lang, Record<JourneyKey, string>> = { en, fr }

/** The Journey page's text getter for one language. */
export function journeyText(lang: Lang): (key: JourneyKey) => string {
  return (key) => journeyMessages[lang][key]
}
