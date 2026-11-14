import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-14',
  slug: 'second-souffle',
  title: 'Second souffle',
  commit: '9e1eaf8',
  genre: { en: 'Melodic electronic instrumental that ends once, then starts again', fr: 'Instrumental électronique mélodique qui finit une fois, puis repart' },
  bpm: 120,
  key: { tonic: 'D', mode: 'minor' },
  bars: 88,
  duration: 179.4,
  model: 'Claude Fable',
  skills: ['music-producer'],
  style: 'drifting-cycles',
  brief: {
    en: 'Style profile “drifting-cycles” and a false-ending form: a short arc rises, comes apart down to one held note, then four bars of nothing. It starts again at half time over a held D in the bass, the chords move above it, a suspended chord waits four bars, and the real peak comes a whole tone higher, in E minor, the bass moving at last on its first beat. 120 bpm. Three constraints: a pedal under moving chords, a late rise of a whole tone, and a five-note theme that is never rewritten, only reharmonised and then handed to the bass.',
    fr: 'Profil de style « drifting-cycles » et une forme à fausse fin : un petit arc monte, se défait jusqu’à une seule note tenue, puis quatre mesures de rien. Il repart en demi-temps sur un Ré tenu à la basse, les accords bougent au-dessus, un accord suspendu attend quatre mesures, et le vrai sommet arrive un ton plus haut, en Mi mineur, la basse bougeant enfin sur son premier temps. 120 bpm. Trois contraintes : une pédale sous des accords qui bougent, une montée tardive d’un ton, et un thème de cinq notes jamais récrit, seulement réharmonisé puis donné à la basse.',
  },
  built: {
    en: 'The eleventh day built through the interface only. About 27 minutes, against 36 the day before, 16 of them for all the notes and the arrangement. 5 tracks, 26 clips (391 notes), 135 placements, 13 devices and 3 automation lanes with 12 points. Every note was typed with step entry. Before publishing, the 21 melodic clips of the saved project were compared with the sheet: 21 out of 21 right. The first WAV export peaked at −6.8 dBFS; with 5 dB more make-up gain on the Compressor, the second is 2:59 with a peak at −1.8 dBFS. The project was saved, reopened with everything back; the .faulkpack was built afterwards from the saved project folder (81 files, none missing), not exported by the app. Measured on the WAV, not listened to: the second peak is not louder than the first.',
    fr: 'Onzième jour construit par l’interface seule. Environ 27 minutes, contre 36 la veille, dont 16 pour toutes les notes et l’arrangement. 5 pistes, 26 clips (391 notes), 135 placements, 13 devices et 3 couloirs d’automation de 12 points. Toutes les notes ont été tapées en saisie pas à pas. Avant la publication, les 21 clips mélodiques du projet enregistré ont été comparés à la fiche : 21 sur 21 justes. Le premier export WAV avait sa crête à −6,8 dBFS ; avec 5 dB de compensation en plus sur le Compressor, le second dure 2:59, crête à −1,8 dBFS. Le projet enregistré, rouvert avec tout revenu ; le .faulkpack a été fabriqué ensuite à partir du dossier enregistré (81 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le WAV, sans écoute : le second sommet n’est pas plus fort que le premier.',
  },
  tested: [
    {
      en: 'Step entry in the piano roll (#886): all 391 notes, about 745 keystrokes for 331 melodic notes, against about 1,090 actions for 328 the day before',
      fr: 'La saisie pas à pas dans le piano roll (#886) : les 391 notes, environ 745 frappes pour 331 notes mélodiques, contre environ 1 090 actions pour 328 la veille',
    },
    {
      en: 'Automation curve shapes (#885): the four shapes set from the ramp dialog; on the logarithmic cutoff the lane read 699 Hz where 698 were expected',
      fr: 'Les formes de courbe d’automation (#885) : les quatre formes posées par le dialogue de rampe ; sur la coupure logarithmique, le couloir lisait 699 Hz pour 698 attendus',
    },
    {
      en: 'Alt+N, the Devices panel following the track and the top bar at 800 px: 26 clips created at the playhead, the panel followed except once, the bar held',
      fr: 'Alt+N, le panneau Devices qui suit la piste et la barre du haut à 800 px : 26 clips créés à la tête de lecture, le panneau a suivi sauf une fois, la barre a tenu',
    },
  ],
  gaps: [
    {
      en: 'Step entry switches off without a word when another track’s clip opens: once, 46 letters played the instrument instead of writing notes (#899).',
      fr: 'La saisie pas à pas se coupe sans le dire quand le clip d’une autre piste s’ouvre : une fois, 46 lettres ont joué l’instrument au lieu d’écrire des notes (#899).',
    },
    {
      en: 'An empty clip made with Alt+N keeps one bar when its placement is made longer: 17 clips out of 26 needed their length set apart (#898).',
      fr: 'Un clip vide créé par Alt+N garde une mesure quand on allonge son placement : 17 clips sur 26 ont demandé leur longueur à part (#898).',
    },
    {
      en: 'The song’s level is known only after the export: the first one came out 6.8 dB under the ceiling with nothing saying so before (#253).',
      fr: 'Le niveau du morceau n’est connu qu’après l’export : le premier est sorti 6,8 dB sous le plafond sans que rien ne le dise avant (#253).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-6874571336',
  tracks: [
    { name: 'Arp', color: '#f08098' },
    { name: 'Basse', color: '#f18a76' },
    { name: 'Nappe', color: '#ea9356' },
    { name: 'Thème', color: '#d99e3f' },
    { name: 'Drums', color: '#c1ab3c' },
  ],
  sections: [
    { name: { en: 'Intro', fr: 'Intro' }, bars: 8, parts: 3 },
    { name: { en: 'A', fr: 'A' }, bars: 16, parts: 6 },
    { name: { en: 'Peak 1', fr: 'Sommet 1' }, bars: 8, parts: 5 },
    { name: { en: 'Dissolution and silence', fr: 'Dissolution et silence' }, bars: 12, parts: 6 },
    { name: { en: 'Return', fr: 'Reprise' }, bars: 16, parts: 6 },
    { name: { en: 'Peak 2', fr: 'Sommet 2' }, bars: 24, parts: 7 },
    { name: { en: 'End', fr: 'Fin' }, bars: 4, parts: 1 },
  ],
} satisfies JourneyEntry
