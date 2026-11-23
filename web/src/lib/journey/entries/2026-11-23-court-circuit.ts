import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-23',
  slug: 'court-circuit',
  title: 'Court-circuit',
  commit: '1a9d424',
  genre: {
    en: 'Dark, warm electronic instrumental carried by the bass, one loop cut in half',
    fr: 'Instrumental électronique sombre et chaud, porté par la basse, une boucle coupée en deux',
  },
  bpm: 116,
  key: { tonic: 'F', mode: 'minor' },
  bars: 96,
  duration: 201.07,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'morzak',
  brief: {
    en: 'The morzak style profile, and a loop with one interruption: a four-bar loop in F minor held for forty bars that changes only its timbre, cut on the third beat of bar 40; four bars of something else, a free phrase in triplets on G♭ maj7, alone; then the loop comes back on other chords, F major then F minor, the chord that turns its third over, and only then does a melody sing. 116 bpm. Three constraints: two cycles that take turns by section over a groove that never changes; the cycle stays and the timbre moves (in the first part, after bar 9 nothing comes in or goes out, only two filter cutoffs move); and the arpeggio is never written: the Arp clips hold only sustained chords, and Cascade plays the arpeggio.',
    fr: 'Le profil de style morzak, et une boucle avec une interruption : une boucle de quatre mesures en Fa mineur tenue quarante mesures, qui ne change que de timbre, coupée au troisième temps de la mesure 40 ; quatre mesures d’autre chose, une phrase libre en triolets sur Sol♭ maj7, seule ; puis la boucle revient sur d’autres accords, Fa majeur puis Fa mineur, l’accord qui retourne sa tierce, et c’est alors seulement qu’une mélodie chante. 116 bpm. Trois contraintes : deux cycles qui alternent par section sur un groove qui ne change pas ; le cycle reste, le timbre bouge (dans la première partie, après la mesure 9 rien n’entre ni ne sort, seules deux coupures de filtre bougent) ; et l’arpège n’est jamais écrit : les clips de la piste Arp ne contiennent que des accords tenus, et Cascade joue l’arpège.',
  },
  built: {
    en: 'The nineteenth day built through the interface only, the whole sheet with nothing left outside Faulk. 20 and a half minutes from the first action to the first export. 7 tracks, 16 clips (218 notes), 20 placements, 23 devices, and 4 automation lanes with 17 points. The 218 notes were typed with step entry. Before publishing, the 11 melodic clips of the saved project were compared with the sheet: 11 out of 11 right, and the hits of the 5 drum clips match it too. The first application of the new lead check: each of the two leads was measured with an export where it is muted, at the end of the filter opening for the sawtooth one, in three rounds of three exports; the kept render adds 1.4 dB for the sine lead and 1.2 dB for the sawtooth at the end of its opening, which goes from 700 Hz to 2.8 kHz and is offset by 2 dB of volume. The final export: 3:21, with a peak at −0.3 dBFS and a crest factor of 15.9 dB. The project was saved and reopened, identical; the .faulkpack was built afterwards from the saved project folder (74 files, none missing), not exported by the app. Measured on the file, not listened to: the level sits near −16 dBFS RMS through the loop, drops to −21.8 on the interruption and rises to −15.1 when the melody sings.',
    fr: 'Dix-neuvième jour construit par l’interface seule, la fiche entière sans rien laisser hors de Faulk. 20 minutes et demie de la première action au premier export. 7 pistes, 16 clips (218 notes), 20 placements, 23 devices et 4 couloirs d’automation de 17 points. Les 218 notes ont été saisies pas à pas. Avant la publication, les 11 clips mélodiques du projet enregistré ont été comparés à la fiche : 11 sur 11 justes, et les coups des 5 clips de batterie y correspondent aussi. Première application du nouveau lead check : chacun des deux leads a été mesuré avec un export où il est coupé, en fin d’ouverture du filtre pour celui en dent de scie, en trois tours de trois exports ; le rendu gardé ajoute 1,4 dB pour le lead sinus et 1,2 dB pour la dent de scie en fin d’ouverture, qui va de 700 Hz à 2,8 kHz et est compensée par 2 dB de volume. L’export final : 3:21, avec une crête à −0,3 dBFS et un facteur de crête de 15,9 dB. Le projet a été enregistré puis rouvert, identique ; le .faulkpack a été construit ensuite à partir du dossier enregistré (74 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, pas écouté : le niveau reste autour de −16 dBFS RMS pendant la boucle, tombe à −21,8 sur l’interruption et monte à −15,1 quand la mélodie chante.',
  },
  tested: [
    {
      en: 'Cascade (#968): 24 chord notes give more than 90 bars of arpeggio, sixteenth notes, up and down over two octaves, present in the export',
      fr: 'Cascade (#968) : 24 notes d’accords donnent plus de 90 mesures d’arpège, en doubles croches, montant et descendant sur deux octaves, présent dans l’export',
    },
    {
      en: 'Sluice (#969) keyed by another track: the pad opens under each hi-hat (−30 dBFS against −37 between two), even when the hi-hat is silenced by a solo',
      fr: 'Sluice (#969) déclenché par une autre piste : la nappe s’ouvre sous chaque charley (−30 dBFS contre −37 entre deux), même quand le charley est rendu muet par un solo',
    },
    {
      en: 'The lead check of #971: every lead measured, at the end of a filter opening, with the opening offset by a volume dip',
      fr: 'Le lead check de #971 : chaque lead mesuré, en fin d’ouverture de filtre, avec l’ouverture compensée par une baisse de volume',
    },
    {
      en: 'Alt+Shift+← acts on a whole selection when a placement has focus, in one undo: found only after twelve ends had been cut one by one',
      fr: 'Alt+Maj+← agit sur toute une sélection quand le focus est sur un placement, en un seul Annuler : découvert seulement après avoir coupé douze fins une par une',
    },
  ],
  gaps: [
    {
      en: 'Offsetting a filter opening lowers the measured lead at the end of the opening, and keeping 1 dB there pushes the sawtooth lead 3 dB above the bass fader; wide-band RMS does not see brightness (#975).',
      fr: 'Compenser une ouverture de filtre fait baisser le lead mesuré en fin d’ouverture, et y tenir 1 dB pousse le lead en dent de scie 3 dB au-dessus du fader de la basse ; le RMS large bande ne voit pas la brillance (#975).',
    },
    {
      en: 'Arranging from the keyboard: Ctrl+G from the free cursor jumps to the selected track and loses focus, and Alt+Shift+← can shrink a placement to one tick (#973).',
      fr: 'Arranger au clavier : Ctrl+G depuis le curseur libre saute sur la piste sélectionnée et perd le focus, et Alt+Maj+← peut réduire un placement à un tick (#973).',
    },
    {
      en: 'Cascade and Sluice have choice groups with no visible name (#974).',
      fr: 'Cascade et Sluice ont des groupes de choix sans nom visible (#974).',
    },
    {
      en: 'No RMS level over a range in Faulk: the lead check took nine exports and a script outside the app (#201).',
      fr: 'Pas de niveau RMS sur une plage dans Faulk : le lead check a demandé neuf exports et un script hors de l’app (#201).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7095636378',
  tracks: [
    { name: 'Basse', color: '#f08098' },
    { name: 'Batterie', color: '#f18a76' },
    { name: 'Charley', color: '#ea9356' },
    { name: 'Arp', color: '#d99e3f' },
    { name: 'Nappe', color: '#c1ab3c' },
    { name: 'Motif', color: '#a0b751' },
    { name: 'Voix', color: '#79bf72' },
  ],
  sections: [
    { name: { en: 'Loop: intro', fr: 'Boucle : entrée' }, bars: 8, parts: 5 },
    { name: { en: 'Loop', fr: 'Boucle' }, bars: 32, parts: 6 },
    { name: { en: 'Break', fr: 'Rupture' }, bars: 4, parts: 2 },
    { name: { en: 'Return', fr: 'Retour' }, bars: 8, parts: 5 },
    { name: { en: 'Song', fr: 'Chant' }, bars: 16, parts: 6 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 16, parts: 6 },
    { name: { en: 'Exit', fr: 'Sortie' }, bars: 12, parts: 6 },
  ],
} satisfies JourneyEntry
