import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-19',
  slug: 'lame-de-fond',
  title: 'Lame de fond',
  commit: 'bda4e16',
  genre: { en: 'Dark, warm, slow electronic instrumental carried by the low end', fr: 'Instrumental électronique sombre et chaud, lent, porté par le grave' },
  bpm: 84,
  key: { tonic: 'F', mode: 'minor' },
  bars: 96,
  duration: 277.3,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'morzak',
  brief: {
    en: 'Style profile “morzak” and a single crescendo: an arpeggio figure, almost out at first, grows like a groundswell that does not fall back before the end. 84 bpm, F minor, Aeolian then Dorian. Three constraints: from bar 33 to bar 88 the same six tracks play and nothing is stacked, so everything that grows is filter, register, subdivision, echo and the chord that changes; only the hi-hat swings its offbeat; and the sixth degree tells the story, D♭ before bar 33, D until bar 64, then both in the same bar at every turn of the peak, where B♭ major becomes B♭ minor.',
    fr: 'Profil de style « morzak » et un crescendo unique : une figure d’arpège presque éteinte grossit comme une lame de fond qui ne retombe pas avant la fin. 84 bpm, Fa mineur, éolien puis dorien. Trois contraintes : de la mesure 33 à la mesure 88 les six mêmes pistes jouent et rien ne s’empile, tout ce qui grossit est filtre, registre, subdivision, écho et l’accord qui change ; seul le charley balance son contretemps ; et le sixième degré raconte le morceau, Ré♭ avant la mesure 33, Ré jusqu’à la mesure 64, puis les deux dans la même mesure à chaque tour du sommet, où Si♭ majeur devient Si♭ mineur.',
  },
  built: {
    en: 'The sixteenth day built through the interface only, the whole sheet with one gap: the swung hi-hat is at 0.625 of the beat instead of 0.6, because the interface offers no swing setting. 21 minutes from the first action to the export. 6 tracks, 21 clips (437 notes), 23 placements, 14 devices on the tracks plus Compressor and Ceiling on the master, and 5 automation lanes with 26 points, laid in 3 minutes. 373 notes were typed with step entry in 787 keystrokes, among them 37 triplets in one go; the other 64 came from “Place a copy”, moved an octave with three notes lowered. Before publishing, the 16 melodic clips of the saved project were compared with the sheet: 16 out of 16 right, and the hits of the 5 drum clips match it too. One WAV export, right the first time: 4:37 with a peak at −0.3 dBFS. The project was saved and reopened from Recent projects, identical; the .faulkpack was built afterwards from the saved project folder (76 files, none missing), not exported by the app. Measured on the file, not listened to: from −24.3 dBFS RMS for the arpeggio alone, the level climbs section after section to −15.2 on the peak plateau, with one breath at bar 64.',
    fr: 'Seizième jour construit par l’interface seule, la fiche entière avec un écart : le charley balancé est à 0,625 du temps au lieu de 0,6, faute de réglage de swing dans l’interface. 21 minutes de la première action à l’export. 6 pistes, 21 clips (437 notes), 23 placements, 14 devices sur les pistes plus Compressor et Ceiling sur le master, et 5 couloirs d’automation de 26 points, posés en 3 minutes. 373 notes ont été tapées en saisie pas à pas, en 787 frappes, dont 37 triolets d’un trait ; les 64 autres viennent de « Placer une copie », montées d’une octave avec trois notes baissées. Avant la publication, les 16 clips mélodiques du projet enregistré ont été comparés à la fiche : 16 sur 16 justes, et les coups des 5 clips de batterie y sont conformes aussi. Un seul export WAV, bon du premier coup : 4:37, crête à −0,3 dBFS. Le projet enregistré, rouvert par Projets récents à l’identique ; le .faulkpack a été fabriqué ensuite à partir du dossier enregistré (76 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, sans écoute : de −24,3 dBFS RMS pour l’arpège seul, le niveau monte section après section jusqu’à −15,2 sur le plateau du sommet, avec un souffle à la mesure 64.',
  },
  tested: [
    {
      en: '“Place a copy” (#933): one pass of the clip, named “Arp A16 2”, and a refusal that names the clip and bars in the way',
      fr: '« Placer une copie » (#933) : une passe du clip, nommée « Arp A16 2 », et un refus qui nomme le clip et les mesures sur le chemin',
    },
    {
      en: 'History lines that name what they act on (#934): “Create Track 2 (Oscillator)”, “Rename Track 6 to Charley”, the automation lane of each ramp',
      fr: 'Des lignes d’historique qui nomment leur objet (#934) : « Créer Piste 2 (Oscillator) », « Renommer Piste 6 en Charley », le couloir d’automation de chaque rampe',
    },
    {
      en: 'Scale and tonic as lists in the command palette, with the transport’s labels in both languages (#935)',
      fr: 'Gamme et tonique en listes dans la palette de commandes, aux libellés du transport dans les deux langues (#935)',
    },
  ],
  gaps: [
    {
      en: 'No swing: a single swung part cannot be written, step entry reaches 0.625 or 0.667 of the beat, not 0.6 (#168).',
      fr: 'Pas de swing : une partie balancée seule ne s’écrit pas, la saisie pas à pas atteint 0,625 ou 0,667 du temps, pas 0,6 (#168).',
    },
    {
      en: 'The piano roll names every black key with a sharp, even in F minor: G♯, A♯, C♯, D♯ where the sheet reads A♭, B♭, D♭, E♭ (#943).',
      fr: 'Le piano roll nomme toutes les touches noires en dièses, même en Fa mineur : G♯, A♯, C♯, D♯ là où la fiche lit La♭, Si♭, Ré♭, Mi♭ (#943).',
    },
    {
      en: 'No gesture goes from the piano roll back to the open clip’s placement: about twelve actions to cut the end of a placement (#944).',
      fr: 'Aucun geste ne ramène du piano roll au placement du clip ouvert : une douzaine d’actions pour couper la fin d’un placement (#944).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7004048308',
  tracks: [
    { name: 'Arp', color: '#d99e3f' },
    { name: 'Basse', color: '#b299f3' },
    { name: 'Nappe', color: '#6bafff' },
    { name: 'Chant', color: '#4ac495' },
    { name: 'Batterie', color: '#f18a76' },
    { name: 'Charley', color: '#a0b751' },
  ],
  sections: [
    { name: { en: 'Seed 1', fr: 'Graine 1' }, bars: 8, parts: 1 },
    { name: { en: 'Seed 2', fr: 'Graine 2' }, bars: 8, parts: 3 },
    { name: { en: 'Seed 3', fr: 'Graine 3' }, bars: 16, parts: 5 },
    { name: { en: 'Rise 1', fr: 'Montée 1' }, bars: 16, parts: 6 },
    { name: { en: 'Rise 2', fr: 'Montée 2' }, bars: 16, parts: 6 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 24, parts: 6 },
    { name: { en: 'End', fr: 'Fin' }, bars: 8, parts: 2 },
  ],
} satisfies JourneyEntry
