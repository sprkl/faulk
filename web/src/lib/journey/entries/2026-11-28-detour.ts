import type { JourneyEntry } from '../types'

export default {
  date: '2026-11-28',
  slug: 'detour',
  title: 'Détour',
  commit: '6684404',
  genre: {
    en: 'Pensive melodic electronic instrumental whose middle section steps into another metre',
    fr: 'Instrumental électronique mélodique et pensif, dont la section du milieu passe dans une autre mesure',
  },
  bpm: 108,
  key: { tonic: 'E', mode: 'minor' },
  bars: 72,
  duration: 163.8,
  model: 'Claude Fable',
  skills: ['music-producer'],
  brief: {
    en: 'No style profile, and a form whose middle part lives in another metre: A in 4/4, B in 6/8, A again. The eighth note never changes speed, only its grouping does, by two in A and by three in B. B visits the relative major, G, with the drums reduced to a pulse, the chords turned into an arpeggio and the melody in long values on a bell; a pad is born there, behind a band-pass filter that slides from 500 Hz to 2 kHz. When A comes back, the pad stays, and the bell keeps the cell of B in dotted quarter notes over the 4/4. 108 bpm, E minor. Three constraints: the eighth note never changes speed; one three-note cell, B, D, E, opens both melodies; and A holds three parts until bar 8, with no pad before B.',
    fr: 'Pas de profil de style, et une forme dont la partie du milieu vit dans une autre mesure : A en 4/4, B en 6/8, puis A encore. La croche ne change jamais de vitesse, seul son groupement change, par deux en A, par trois en B. B visite le relatif majeur, Sol, la batterie réduite à un pouls, les accords devenus arpège et la mélodie en valeurs longues sur une cloche ; une nappe y naît, derrière un filtre passe-bande qui glisse de 500 Hz à 2 kHz. Au retour de A, la nappe reste, et la cloche garde la cellule de B en noires pointées par-dessus le 4/4. 108 bpm, Mi mineur. Trois contraintes : la croche ne change jamais de vitesse ; une seule cellule de trois notes, Si, Ré, Mi, ouvre les deux mélodies ; et A tient en trois parties jusqu’à la mesure 8, sans nappe avant B.',
  },
  built: {
    en: 'The twenty-fourth day built through the interface only: the whole sheet except the metre of B. Faulk holds one time signature per project, so the project stayed in 4/4 and B was written in eighth notes across 24 bars of 4/4, the 32 bars of 6/8 it plays (#990). 20 minutes from the first action to the first export, 11 of them for the notes and the placements. 7 tracks, 16 clips (243 notes), 22 placements, 22 devices (two of them on the master), 5 automation lanes with 13 points. Before publishing, the 11 melodic clips of the saved project were compared with the sheet: 11 out of 11 right, and the hits of the 5 drum clips match it too. The lead check measured both leads with an export where each is muted, without raising a lead above the bass fader. The square lead adds 1.3 dB at the end of its filter opening, bounded to 0.74 octave so that no volume dip was needed, and 1.9 dB with its filter closed. The bell adds 1.1 dB where it leads in B and 3.5 dB over the pad and arpeggio alone. Over bars 49–56, alone on the full groove, it reads 0.7 dB, buried by the rule; raising it further would have passed the bass fader, so it was left there. A first round had it at 4.2 dB in the opening cycle of B: the pad was raised there by a volume lane, the bell’s sustain raised from 25 % to 60 % and its fader lowered. The final export: 2:44, with a peak at −0.3 dBFS and −17.3 dBFS RMS. The project was saved and reopened, identical file by file; the .faulkpack was built afterwards from the saved project folder (75 files, none missing), not exported by the app. Measured on the file, not listened to: the opening cycle of B drops to −21.2 dBFS RMS, and the return of A with the song, bars 57–68, sits 0.3 dB above the first time, bars 9–24.',
    fr: 'Vingt-quatrième jour construit par l’interface seule : la fiche entière, sauf la mesure de B. Faulk n’a qu’une signature rythmique par projet : le projet est resté en 4/4, et B a été écrit en croches sur 24 mesures de 4/4, les 32 mesures de 6/8 qu’il joue (#990). 20 minutes de la première action au premier export, dont 11 pour les notes et les placements. 7 pistes, 16 clips (243 notes), 22 placements, 22 devices (dont deux sur le master), 5 couloirs d’automation de 13 points. Avant la publication, les 11 clips mélodiques du projet enregistré ont été comparés à la fiche : 11 sur 11 justes, et les coups des 5 clips de batterie y correspondent aussi. Le lead check a mesuré les deux leads avec un export où chacun est coupé, sans monter un lead au-dessus du fader de la basse. Le lead carré ajoute 1,3 dB en fin d’ouverture de son filtre, bornée à 0,74 octave pour qu’aucune baisse de volume ne soit nécessaire, et 1,9 dB filtre fermé. La cloche ajoute 1,1 dB là où elle mène en B, et 3,5 dB sur la nappe et l’arpège seuls. Sur les mesures 49–56, seule sur le groove complet, elle se lit à 0,7 dB, enterrée selon la règle ; la monter encore l’aurait fait passer au-dessus du fader de la basse, elle est donc restée là. Un premier tour la donnait à 4,2 dB dans le premier cycle de B : la nappe y a été montée par un couloir de volume, le maintien de la cloche monté de 25 % à 60 % et son fader baissé. L’export final : 2:44, avec une crête à −0,3 dBFS et −17,3 dBFS RMS. Le projet a été enregistré puis rouvert, identique fichier par fichier ; le .faulkpack a été construit ensuite à partir du dossier enregistré (75 fichiers, aucun manquant), non exporté par l’app. Mesuré sur le fichier, pas écouté : le premier cycle de B tombe à −21,2 dBFS RMS, et le retour de A avec le chant, mesures 57–68, se tient 0,3 dB au-dessus de la première fois, mesures 9–24.',
  },
  tested: [
    {
      en: 'A section in another time signature, tried for the first time: B in 6/8 inside a piece in 4/4, then the whole project switched to 6/8 with “Change the time signature” and put back with two Undo (#990)',
      fr: 'Une section dans une autre signature, essayée pour la première fois : B en 6/8 au milieu d’un morceau en 4/4, puis le projet entier passé en 6/8 par « Changer la signature rythmique » et remis par deux Annuler (#990)',
    },
    {
      en: 'The Filter in Band-pass mode, with a lane on its Resonance: the values are typed in percent and the lane follows during playback',
      fr: 'Le Filter en mode Passe-bande, avec un couloir sur sa Résonance : les valeurs se tapent en pour cent et le couloir suit pendant la lecture',
    },
    {
      en: 'Swing’s Humanize time, Humanize velocity and Seed on the hi-hat: set, but nothing on screen shows what they do, and their effect was not measured',
      fr: 'L’Humanisation du temps, l’Humanisation de la vélocité et la Graine de Swing sur le charley : réglées, mais rien à l’écran ne montre ce qu’elles font, et leur effet n’a pas été mesuré',
    },
  ],
  gaps: [
    {
      en: 'A time signature cannot be set at a bar: during B the ruler, the bar numbers, “Go to…”, the piano roll and the transport count in 4/4, and switching the project to 6/8 makes A wrong instead (#990).',
      fr: 'Une signature ne peut pas se poser à une mesure : pendant B, la règle, les numéros de mesure, « Aller à… », le piano roll et le transport comptent en 4/4, et passer le projet en 6/8 rend A faux à la place (#990).',
    },
    {
      en: 'Every “Go to…”, lengthening and ramp of B had to be converted by hand from bars of 6/8 to bars of 4/4.',
      fr: 'Chaque « Aller à… », chaque allongement et chaque rampe de B a dû être converti à la main des mesures de 6/8 en mesures de 4/4.',
    },
    {
      en: 'The lead check and the levelling still took six exports for two leads (#201).',
      fr: 'Le lead check et la mise à niveau ont encore demandé six exports pour deux leads (#201).',
    },
  ],
  journal: 'https://github.com/sprkl/faulk/issues/362#issuecomment-7236596831',
  tracks: [
    { name: 'Basse', color: '#f08098' },
    { name: 'Batterie', color: '#f18a76' },
    { name: 'Charley', color: '#ea9356' },
    { name: 'Accords', color: '#d99e3f' },
    { name: 'Nappe', color: '#c1ab3c' },
    { name: 'Chant', color: '#a0b751' },
    { name: 'Cloche', color: '#79bf72' },
  ],
  sections: [
    { name: { en: 'The groove, in three parts', fr: 'Le groove, en trois parties' }, bars: 8, parts: 4 },
    { name: { en: 'The song comes in', fr: 'Le chant entre' }, bars: 16, parts: 5 },
    { name: { en: 'Detour in 6/8, in the relative major', fr: 'Détour en 6/8, dans le relatif majeur' }, bars: 24, parts: 7 },
    { name: { en: 'Back in 4/4, the bell against the beat', fr: 'Retour en 4/4, la cloche contre le temps' }, bars: 8, parts: 6 },
    { name: { en: 'The song over the bell, then the last chord', fr: 'Le chant sur la cloche, puis le dernier accord' }, bars: 16, parts: 9 },
  ],
} satisfies JourneyEntry
