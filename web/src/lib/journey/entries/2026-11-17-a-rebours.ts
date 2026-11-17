import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-17',
  slug: 'a-rebours',
  title: 'À rebours',
  commit: '36c90d3',
  genre: { en: 'Bright, then nostalgic melodic electronic instrumental', fr: 'Instrumental électronique mélodique, lumineux puis nostalgique' },
  bpm: 108,
  key: { tonic: 'F', mode: 'major' },
  bars: 80,
  duration: 181.2,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'drifting-cycles',
  brief: {
    en: 'Style profile “drifting-cycles” and an inverted arc: the track opens on its peak, strips down, grows back, then peaks again on a new image of chords, one tone higher. 108 bpm, F major, rising to G major at bar 53. Three constraints: the seven-note hook never changes its notes, only the chords under it change, apart from the one rise of a tone; a bass pedal on F from bar 25, then a late rise at 65 % after a thinned bar, with a lead note held across the cut; and no chord cycle serves two sections.',
    fr: 'Profil de style « drifting-cycles » et un arc inversé : le morceau s’ouvre sur son sommet, se dépouille, repousse, puis culmine de nouveau sur une autre image d’accords, un ton plus haut. 108 bpm, Fa majeur, hausse en Sol majeur à la mesure 53. Trois contraintes : le motif de sept notes ne change jamais ses notes, seuls les accords dessous changent, hormis la hausse d’un ton ; une pédale de Fa à la basse dès la mesure 25, puis une hausse tardive à 65 % après une mesure amincie, avec une note du lead tenue à travers la coupe ; et aucun cycle d’accords ne sert à deux sections.',
  },
  built: {
    en: 'The fourteenth day built through the interface only, the whole sheet with no exception. About 23 minutes from opening the app to the final export, 14 of them for all the notes and the arrangement. 5 tracks, 23 clips (370 notes), 73 placements, 14 devices on the tracks plus Compressor and Ceiling on the master, and 3 automation lanes with 16 points. 309 notes were typed with step entry, about 770 keystrokes and one single Alt+E for the whole session; the 61 notes of the four raised patterns were copied and transposed a tone up. Before publishing, the 19 melodic clips of the saved project were compared with the sheet: 19 out of 19 right, and the hits of the 4 drum clips match it too. Two WAV exports: the first was too heavy in the bass, which was brought down 7 dB; the second is 3:01 with a peak at −0.3 dBFS. The project was saved and reopened with everything back; the .faulkpack was built afterwards from the saved project folder (76 files, none missing), not exported by the app. Measured on the file, not listened to: the strip-down sits 4.5 dB under the opening peak, and the raised peak with the sung phrase is the loudest, by only 0.2 dB.',
    fr: 'Quatorzième jour construit par l’interface seule, la fiche entière sans exception. Environ 23 minutes de l’ouverture de l’app à l’export final, dont 14 pour toutes les notes et l’arrangement. 5 pistes, 23 clips (370 notes), 73 placements, 14 devices sur les pistes plus Compressor et Ceiling sur le master, et 3 couloirs d’automation de 16 points. 309 notes ont été tapées en saisie pas à pas, environ 770 frappes et un seul Alt+E pour toute la séance ; les 61 notes des quatre motifs haussés ont été copiées puis transposées d’un ton. Avant la publication, les 19 clips mélodiques du projet enregistré ont été comparés à la fiche : 19 sur 19 justes, et les coups des 4 clips de batterie y sont conformes aussi. Deux exports WAV : le premier était trop chargé en basse, baissée de 7 dB ; le second dure 3:01, crête à −0,3 dBFS. Le projet enregistré, rouvert avec tout revenu ; le .faulkpack a été fabriqué ensuite à partir du dossier enregistré (76 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, sans écoute : le dépouillement est 4,5 dB sous le sommet d’ouverture, et le sommet haussé avec la phrase chantée est le plus fort, de 0,2 dB seulement.',
  },
  tested: [
    {
      en: '“Lengthen to…” (#920), used 21 times: an empty clip of 8 bars in one gesture, and the drums and the pedal bass as single placements that replay their content, also from the piano roll',
      fr: '« Allonger jusqu’à… » (#920), utilisé 21 fois : un clip vide de 8 mesures en un geste, et la batterie et la basse pédale en placements uniques qui rejouent leur contenu, aussi depuis le piano roll',
    },
    {
      en: 'Step entry stays armed (#919): one Alt+E for five tracks, through Ctrl+G, Alt+N and the Clips tab',
      fr: 'La saisie pas à pas reste armée (#919) : un seul Alt+E pour cinq pistes, à travers Ctrl+G, Alt+N et l’onglet Clips',
    },
    {
      en: 'The “Ramp to…” dialog (#921): an empty end value is refused, the curve is kept, the playhead moves to the end point, so ramps follow each other without Ctrl+G',
      fr: 'Le dialogue « Rampe jusqu’à… » (#921) : une valeur d’arrivée vide est refusée, la courbe est gardée, la tête de lecture va au point d’arrivée, et les rampes s’enchaînent sans Ctrl+G',
    },
    {
      en: 'The command palette (#917, #922): tempo and project name set with Ctrl+K, Ctrl+S and Ctrl+Shift+S shown next to their commands',
      fr: 'La palette de commandes (#917, #922) : tempo et nom du projet réglés par Ctrl+K, Ctrl+S et Ctrl+Maj+S affichés à côté de leurs commandes',
    },
  ],
  gaps: [
    {
      en: 'Past a few commands, the palette speaks the agents’ language: clip ids and JSON to type, field names such as `bpm` or `tick`, English summaries written for agents, and the interface’s own gestures cannot be found (#926).',
      fr: 'Au-delà de quelques commandes, la palette parle la langue des agents : identifiants de clip et JSON à taper, champs nommés `bpm` ou `tick`, résumés en anglais écrits pour les agents, et les gestes de l’interface introuvables (#926).',
    },
    {
      en: '“Automate: Volume” from the mixer puts the arrangement’s snap back to “Auto” without a word (#925).',
      fr: '« Automatiser : Volume » depuis le mixeur remet le magnétisme de l’arrangement sur « Auto » sans rien dire (#925).',
    },
    {
      en: 'The level is only known after the export, and the factory levels of Oscillator and Drums are about ten decibels apart: two exports to balance the bass (#253).',
      fr: 'Le niveau ne se connaît qu’après l’export, et les niveaux d’usine d’Oscillator et de Drums sont écartés d’une dizaine de décibels : deux exports pour doser la basse (#253).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6942949797',
  tracks: [
    { name: 'Lead', color: '#f08098' },
    { name: 'Pluck', color: '#f18a76' },
    { name: 'Nappe', color: '#ea9356' },
    { name: 'Basse', color: '#d99e3f' },
    { name: 'Drums', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 16, parts: 5 },
    { name: { en: 'Strip-down', fr: 'Dépouillement' }, bars: 8, parts: 2 },
    { name: { en: 'Regrowth', fr: 'Repousse' }, bars: 8, parts: 3 },
    { name: { en: 'Regrowth 2', fr: 'Repousse 2' }, bars: 8, parts: 6 },
    { name: { en: 'Peak II', fr: 'Sommet II' }, bars: 8, parts: 5 },
    { name: { en: 'Turn', fr: 'Bascule' }, bars: 8, parts: 8 },
    { name: { en: 'Raised peak', fr: 'Sommet haussé' }, bars: 8, parts: 5 },
    { name: { en: 'Dissolution', fr: 'Dissolution' }, bars: 12, parts: 5 },
    { name: { en: 'Last voice', fr: 'Dernière voix' }, bars: 4, parts: 2 },
  ],
} satisfies JourneyEntry
