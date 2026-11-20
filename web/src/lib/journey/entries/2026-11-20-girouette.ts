import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-20',
  slug: 'girouette',
  title: 'Girouette',
  commit: '2635648',
  genre: { en: 'Bright, melodic electronic instrumental, danceable without hardness', fr: 'Instrumental électronique mélodique, clair, dansant sans dureté' },
  bpm: 122,
  key: { tonic: 'D', mode: 'major' },
  bars: 96,
  duration: 192.0,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'drifting-cycles',
  brief: {
    en: 'Style profile “drifting-cycles” and a theme with variations: a weather vane does not turn, the wind does. An eight-note hook is stated early and never changes its notes; at each section the chords, the bass and the rhythm turn around it. 122 bpm, D major, a visit to B minor in the middle, then a rise of a minor third to F major at bar 65. Three constraints: every section has its own chord cycle, which keeps one chord of the previous one as an anchor, so the hook is reharmonised at each return; one pedal, one suspended chord, one rise: from bar 49 to 64 the bass holds D under moving chords, and at 65 it moves for the first time, the suspended G falling at last onto F natural; and only the hi-hat swings.',
    fr: 'Profil de style « drifting-cycles » et un thème avec variations : une girouette ne change pas, c’est le vent qui tourne. Une accroche de huit notes est dite tôt et ne change plus de notes ; à chaque section ce sont les accords, la basse et le rythme qui tournent autour d’elle. 122 bpm, Ré majeur, une visite de Si mineur au milieu, puis une hausse d’une tierce mineure vers Fa majeur à la mesure 65. Trois contraintes : chaque section a son cycle d’accords, qui garde un accord du précédent comme ancre, si bien que l’accroche est réharmonisée à chaque reprise ; une pédale, un accord suspendu, une seule hausse : de la mesure 49 à 64 la basse tient Ré sous des accords qui bougent, et à 65 elle bouge pour la première fois, le Sol suspendu descendant enfin sur Fa naturel ; et seul le charley balance.',
  },
  built: {
    en: 'The seventeenth day built through the interface only, the whole sheet with three gaps: the swung hi-hat is at 66.7 % instead of 58 %, because the interface offers no swing setting, and the sidechain pumping and the noise rise the profile asks for were left out, for want of an external sidechain and a noise source. 23 and a half minutes from the first action to the export. 7 tracks, 26 clips (320 notes), 33 placements, 16 devices on the tracks plus Compressor and Ceiling on the master, and 4 automation lanes with 25 points. 280 notes were typed with step entry in 699 keystrokes; the other 40 came from transposed copies, one note corrected. Before publishing, the 21 melodic clips of the saved project were compared with the sheet: 21 out of 21 right, and the hits of the 5 drum clips match it too. One WAV export, right the first time: 3:12 with a peak at −0.3 dBFS. The project was saved and reopened from Recent projects, identical; the .faulkpack was built afterwards from the saved project folder (89 files, none missing), not exported by the app. Measured on the file, not listened to: from −19.2 dBFS RMS at the start, the level reaches −15.6 on the theme, dips to −17.0 in the B minor variation, and the peak after the rise, at −15.2, is only 0.1 dB above the pedal variation.',
    fr: 'Dix-septième jour construit par l’interface seule, la fiche entière avec trois écarts : le charley balancé est à 66,7 % au lieu de 58 %, faute de réglage de swing dans l’interface, et le pompage par sidechain et la montée de bruit que demande le profil sont laissés de côté, faute de sidechain externe et de source de bruit. 23 minutes et demie de la première action à l’export. 7 pistes, 26 clips (320 notes), 33 placements, 16 devices sur les pistes plus Compressor et Ceiling sur le master, et 4 couloirs d’automation de 25 points. 280 notes ont été tapées en saisie pas à pas, en 699 frappes ; les 40 autres viennent de copies transposées, une note corrigée. Avant la publication, les 21 clips mélodiques du projet enregistré ont été comparés à la fiche : 21 sur 21 justes, et les coups des 5 clips de batterie y sont conformes aussi. Un seul export WAV, bon du premier coup : 3:12, crête à −0,3 dBFS. Le projet enregistré, rouvert par Projets récents à l’identique ; le .faulkpack a été fabriqué ensuite à partir du dossier enregistré (89 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, sans écoute : de −19,2 dBFS RMS au début, le niveau atteint −15,6 sur le thème, redescend à −17,0 dans la variation en Si mineur, et le sommet après la hausse, à −15,2, ne dépasse la variation de la pédale que de 0,1 dB.',
  },
  tested: [
    {
      en: 'Note names that follow the key (#943): “F♯5” in D major, and a B♭ read “A♯4” in D major, “B♭4” once the project is in F major',
      fr: 'Des noms de notes qui suivent la tonalité (#943) : « F♯5 » en Ré majeur, et un Si♭ lu « A♯4 » en Ré majeur, « B♭4 » une fois le projet en Fa majeur',
    },
    {
      en: 'Alt+P from the piano roll to the open clip’s placement, selected in the arrangement (#944)',
      fr: 'Alt+P depuis le piano roll jusqu’au placement du clip ouvert, sélectionné dans l’arrangement (#944)',
    },
  ],
  gaps: [
    {
      en: 'No swing: the hi-hat asked for sixteenths at 58 % and step entry reaches 66.7 % with sextuplets (#168).',
      fr: 'Pas de swing : le charley demandait des doubles croches à 58 %, la saisie pas à pas atteint 66,7 % par les sextolets (#168).',
    },
    {
      en: 'The Compressor only listens to its own track: no pumping triggered by the kick (#952).',
      fr: 'Le Compressor n’écoute que sa propre piste : pas de pompage déclenché par la grosse caisse (#952).',
    },
    {
      en: 'One key per project: from bar 65, in F major, the piano roll still calls B♭ “A♯” (#953).',
      fr: 'Une seule tonalité par projet : à partir de la mesure 65, en Fa majeur, le piano roll appelle encore le Si♭ « A♯ » (#953).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7034427143',
  tracks: [
    { name: 'Accroche', color: '#d99e3f' },
    { name: 'Arpège', color: '#a0b751' },
    { name: 'Nappe', color: '#6bafff' },
    { name: 'Basse', color: '#b299f3' },
    { name: 'Batterie', color: '#f18a76' },
    { name: 'Charley', color: '#c1ab3c' },
    { name: 'Chant', color: '#4ac495' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 3 },
    { name: { en: 'Theme', fr: 'Thème' }, bars: 8, parts: 5 },
    { name: { en: 'Variation 1', fr: 'Variation 1' }, bars: 16, parts: 7 },
    { name: { en: 'Variation 2', fr: 'Variation 2' }, bars: 16, parts: 5 },
    { name: { en: 'Variation 3', fr: 'Variation 3' }, bars: 16, parts: 5 },
    { name: { en: 'Peak', fr: 'Sommet' }, bars: 16, parts: 7 },
    { name: { en: 'Return', fr: 'Retour' }, bars: 8, parts: 6 },
    { name: { en: 'Coda', fr: 'Coda' }, bars: 4, parts: 3 },
    { name: { en: 'End', fr: 'Fin' }, bars: 4, parts: 2 },
  ],
} satisfies JourneyEntry
