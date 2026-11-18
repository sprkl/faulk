import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-18',
  slug: 'manege',
  title: 'Manège',
  commit: 'a950b11',
  genre: { en: 'Upbeat electronic instrumental, a song without words', fr: 'Instrumental électronique enjoué, chanson sans paroles' },
  bpm: 116,
  key: { tonic: 'A', mode: 'major' },
  bars: 80,
  duration: 169.0,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'No style profile, a rondo (A B A C A): a chorus that comes back three times and brings something back from each visit, like a merry-go-round. 116 bpm, A major, with a visit to A harmonic minor in B, the E major chord common to both opening the door back. Three constraints: the chorus, its bass and its chords keep their notes at every return, and what changes comes from the visits (the octave after B, the hi-hat in threes and the bell after C); the chorus holds in five notes, with its peak once per turn; the bass never plays the downbeat, and no seam is a fill, each one is a withdrawal.',
    fr: 'Sans profil de style, un rondo (A B A C A) : un refrain qui revient trois fois et rapporte quelque chose de chaque visite, comme un tour de manège. 116 bpm, La majeur, avec une visite en La mineur harmonique dans B, l’accord de Mi majeur commun aux deux ouvrant la porte du retour. Trois contraintes : le refrain, sa basse et ses accords gardent leurs notes à chaque retour, et ce qui change vient des visites (l’octave après B, le charley à trois et la cloche après C) ; le refrain tient en cinq notes, son sommet une fois par tour ; la basse ne joue jamais le premier temps, et aucune couture n’est un roulement, chacune est un retrait.',
  },
  built: {
    en: 'The fifteenth day built through the interface only, the whole sheet with no exception. 19 minutes from the first action to the export. 6 tracks, 15 clips (226 notes), 23 placements, 16 devices on the tracks plus Compressor and Ceiling on the master, and 3 automation lanes with 13 points. 154 notes were typed with step entry in 464 keystrokes; the other 72 were copied and transposed. Before publishing, the 12 melodic clips of the saved project were compared with the sheet: 12 out of 12 right, and the hits of the 3 drum clips match it too. One WAV export, right the first time: 2:49 with a peak at −0.3 dBFS. The project was saved and reopened from Recent projects with everything back; the .faulkpack was built afterwards from the saved project folder (64 files, none missing), not exported by the app. Measured on the file, not listened to: the A sections sit at about −14.6 dBFS RMS, B 6.3 dB lower, the empty bar 48 at −27.9, and the last A, with the chorus doubled, is the loudest at −13.8.',
    fr: 'Quinzième jour construit par l’interface seule, la fiche entière sans exception. 19 minutes de la première action à l’export. 6 pistes, 15 clips (226 notes), 23 placements, 16 devices sur les pistes plus Compressor et Ceiling sur le master, et 3 couloirs d’automation de 13 points. 154 notes ont été tapées en saisie pas à pas, en 464 frappes ; les 72 autres ont été copiées puis transposées. Avant la publication, les 12 clips mélodiques du projet enregistré ont été comparés à la fiche : 12 sur 12 justes, et les coups des 3 clips de batterie y sont conformes aussi. Un seul export WAV, bon du premier coup : 2:49, crête à −0,3 dBFS. Le projet enregistré, rouvert par Projets récents avec tout revenu ; le .faulkpack a été fabriqué ensuite à partir du dossier enregistré (64 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, sans écoute : les A sont à −14,6 dBFS RMS environ, B 6,3 dB plus bas, la mesure 48 vide à −27,9, et le dernier A, refrain doublé, est le plus fort à −13,8.',
  },
  tested: [
    {
      en: 'The command palette for a musician (#930, #931): tempo, project name, scale, track colour and transposition set from it, its forms filled from the open clip',
      fr: 'La palette de commandes pour musicien (#930, #931) : tempo, nom du projet, gamme, couleur de piste et transposition réglés depuis elle, ses formulaires remplis depuis le clip ouvert',
    },
    {
      en: 'The arrangement snap stays on “Bar” after “Automate: Volume” from the mixer (#929)',
      fr: 'Le magnétisme de l’arrangement reste sur « Mesure » après « Automatiser : Volume » depuis le mixeur (#929)',
    },
    {
      en: 'A double click in an empty lane makes a clip and opens it in the piano roll (#924)',
      fr: 'Un double-clic dans une ligne vide crée un clip et l’ouvre dans le piano roll (#924)',
    },
  ],
  gaps: [
    {
      en: '“Place a copy” in the Clips tab takes the length of the source’s first placement, not the clip’s, and is refused where the clip fits: about fifteen actions to make a variant of the chorus (#933).',
      fr: '« Placer une copie » dans l’onglet Clips prend la longueur du premier placement de la source, pas celle du clip, et se fait refuser là où le clip tient : une quinzaine d’actions pour faire une variante du refrain (#933).',
    },
    {
      en: 'Some palette fields still ask for agent values: scale ids in English, a tonic from 0 to 11, a colour in #rrggbb, an instrument or device type in free text, “Type null for none.” (#935).',
      fr: 'Des champs de la palette demandent encore des valeurs d’agent : gammes en identifiants anglais, tonique de 0 à 11, couleur en #rrggbb, instrument ou type de device en texte libre, « Tapez null pour aucun. » (#935).',
    },
    {
      en: 'The history lists lines that do not say what they undo: “Create” five times for five tracks, “Create Clip 1 ()”, scenes and automation points (#934).',
      fr: 'L’historique aligne des lignes qui ne disent pas ce qu’elles annulent : « Créer » cinq fois pour cinq pistes, « Créer Clip 1 () », scènes et points d’automation (#934).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6973171548',
  tracks: [
    { name: 'Lead', color: '#d99e3f' },
    { name: 'Drums', color: '#f18a76' },
    { name: 'Basse', color: '#b299f3' },
    { name: 'Accords', color: '#6bafff' },
    { name: 'Nappe', color: '#79bf72' },
    { name: 'Cloche', color: '#00c5b7' },
  ],
  sections: [
    { name: { en: 'A1', fr: 'A1' }, bars: 16, parts: 4 },
    { name: { en: 'B', fr: 'B' }, bars: 8, parts: 2 },
    { name: { en: 'A2', fr: 'A2' }, bars: 16, parts: 4 },
    { name: { en: 'C', fr: 'C' }, bars: 8, parts: 4 },
    { name: { en: 'A3', fr: 'A3' }, bars: 8, parts: 4 },
    { name: { en: 'A3 high', fr: 'A3 haut' }, bars: 8, parts: 5 },
    { name: { en: 'A3 double', fr: 'A3 doublé' }, bars: 8, parts: 5 },
    { name: { en: 'Coda', fr: 'Coda' }, bars: 8, parts: 2 },
  ],
} satisfies JourneyEntry
