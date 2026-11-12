import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-12',
  slug: 'apnee',
  title: 'Apnée',
  commit: '59e3a48',
  genre: { en: 'Electronic instrumental, a drop built on silence', fr: 'Instrumental électronique, un drop bâti sur le silence' },
  bpm: 122,
  key: { tonic: 'E', mode: 'minor' },
  bars: 72,
  duration: 137.7,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'A drop hinged on silence: the impact comes from the bar before it. A soft, full verse (pad, plucked phrase, held bass, light beat), a build where the phrase shortens, one empty bar, then a drop with fewer parts than the verse: a kick, a sixteenth-note bass and a three-note hook. Three constraints: the drop has fewer parts than the verse, the hook uses only E, G and B, and no hi-hat before the drop. 122 bpm in E minor, no style profile.',
    fr: 'Un drop à charnière de silence : l’impact vient de la mesure d’avant. Un couplet doux et plein (nappe, phrase pincée, basse tenue, pulsation légère), une montée où la phrase se raccourcit, une mesure de vide, puis un drop qui a moins de parties que le couplet : un kick, une basse en doubles croches et un crochet de trois notes. Trois contraintes : le drop a moins de parties que le couplet, le crochet n’a que trois hauteurs (Mi, Sol, Si), aucun charley avant le drop. 122 bpm en Mi mineur, aucun profil de style.',
  },
  built: {
    en: 'The ninth day built through the interface only. 34 minutes, against 50 the day before. 5 tracks, 14 patterns, 142 notes, 106 placements and 3 automation lanes with 8 points. The run used Contour and Ripple, two new effects not yet merged into develop: it ran on a local build of develop 59e3a48 with their branches (#751, #752 and #834) added. The track was written directly in place and repeated, not drafted aside. Before the export, the 12 melodic clips of the saved project were compared with the sheet: 12 out of 12 right. The WAV export is 2:18 and the empty bar measures about ten dB below the verse. The project was saved, reopened with everything back, and exported as a .faulkpack.',
    fr: 'Neuvième jour construit par l’interface seule. 34 minutes, contre 50 la veille. 5 pistes, 14 motifs, 142 notes, 106 placements et 3 couloirs d’automation de 8 points. Le passage a utilisé Contour et Ripple, deux effets nouveaux pas encore fusionnés dans develop : il a tourné sur un build local de develop 59e3a48 auquel s’ajoutaient leurs branches (#751, #752 et #834). Le morceau a été écrit directement à sa place puis répété, sans brouillon à côté. Avant l’export, les 12 clips mélodiques du projet enregistré ont été comparés à la fiche : 12 sur 12 justes. L’export WAV dure 2:18 et la mesure vide mesure environ dix dB sous le couplet. Le projet enregistré, rouvert avec tout revenu, et exporté en .faulkpack.',
  },
  tested: [
    {
      en: 'Contour on the bass, the plucked phrase and the master, with the spectrum drawn behind the curve during playback',
      fr: 'Contour sur la basse, la phrase pincée et le master, avec le spectre affiché derrière la courbe pendant la lecture',
    },
    {
      en: 'Ripple as a slow phaser on the pad and a synced flanger on the hook, its mix automated',
      fr: 'Ripple en phaser lent sur la nappe et en flanger synchronisé sur le crochet, son mélange automatisé',
    },
    {
      en: 'The piano roll keeping its tool and snap from one clip to the next (#841), and Ctrl+D refusing to replace a placement (#844)',
      fr: 'Le piano roll qui garde son outil et son magnétisme d’un clip à l’autre (#841), et Ctrl+D qui refuse de remplacer un placement (#844)',
    },
  ],
  gaps: [
    {
      en: 'The first clip of an empty track can only be created at the free cursor, which follows neither Ctrl+G nor Home: the first four clips were made at bar 51 and moved back (#852).',
      fr: 'Le premier clip d’une piste vide ne se crée qu’au curseur libre, qui ne suit ni Ctrl+G ni Début : les quatre premiers clips ont été créés à la mesure 51 puis replacés (#852).',
    },
    {
      en: 'Placing a copy from the reserve on an occupied spot cut the start of the neighbouring clip without a message (#805).',
      fr: '« Placer une copie » de la réserve sur une place occupée a coupé le début du clip voisin sans le dire (#805).',
    },
    {
      en: 'Contour’s band switches have no label on screen, and a click on a point does not give it the keyboard (#156).',
      fr: 'Les interrupteurs de bande de Contour n’ont pas de légende à l’écran, et un clic sur un point ne lui donne pas le clavier (#156).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6835722840',
  tracks: [
    { name: 'Basse', color: '#f08098' },
    { name: 'Drums', color: '#f18a76' },
    { name: 'Nappe', color: '#ea9356' },
    { name: 'Phrase', color: '#d99e3f' },
    { name: 'Crochet', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Verse', fr: 'Couplet' }, bars: 16, parts: 4 },
    { name: { en: 'Build and empty bar', fr: 'Montée et vide' }, bars: 9, parts: 4 },
    { name: { en: 'Drop', fr: 'Drop' }, bars: 16, parts: 3 },
    { name: { en: 'Verse 2', fr: 'Couplet 2' }, bars: 8, parts: 4 },
    { name: { en: 'Single note', fr: 'Note seule' }, bars: 1, parts: 1 },
    { name: { en: 'Drop 2', fr: 'Drop 2' }, bars: 7, parts: 3 },
    { name: { en: 'Outro', fr: 'Sortie' }, bars: 15, parts: 2 },
  ],
} satisfies JourneyEntry
