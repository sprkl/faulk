/**
 * Labels of the Compressor device's parameters and of their values, keyed `compressor.<key>` (the keys of its
 * descriptor, `DEVICES` in the generated protocol). The device's name is not here: it comes from its
 * descriptor, one English name never translated (founder's decision of 2026-10-14).
 */

import { defineFeature } from './feature'

export const compressor = defineFeature(
  {
    'compressor.threshold': 'Threshold',
    'compressor.ratio': 'Ratio',
    'compressor.knee': 'Knee',
    'compressor.attack': 'Attack',
    'compressor.release_mode': 'Release mode',
    'compressor.manual': 'Manual',
    'compressor.auto': 'Auto',
    'compressor.release': 'Release',
    'compressor.detection': 'Detection',
    'compressor.peak': 'Peak',
    'compressor.rms': 'RMS',
    'compressor.lookahead': 'Lookahead',
    'compressor.sidechain_hpf': 'Sidechain high-pass',
    'compressor.off': 'Off',
    'compressor.on': 'On',
    'compressor.sidechain_freq': 'Sidechain frequency',
    'compressor.makeup': 'Makeup',
    'compressor.mix': 'Mix',
  },
  {
    'compressor.threshold': 'Seuil',
    'compressor.ratio': 'Ratio',
    'compressor.knee': 'Genou',
    'compressor.attack': 'Attaque',
    'compressor.release_mode': 'Mode de relâchement',
    'compressor.manual': 'Manuel',
    'compressor.auto': 'Auto',
    'compressor.release': 'Relâchement',
    'compressor.detection': 'Détection',
    'compressor.peak': 'Crête',
    'compressor.rms': 'RMS',
    'compressor.lookahead': 'Anticipation',
    'compressor.sidechain_hpf': 'Passe-haut du sidechain',
    'compressor.off': 'Non',
    'compressor.on': 'Oui',
    'compressor.sidechain_freq': 'Fréquence du sidechain',
    'compressor.makeup': 'Compensation',
    'compressor.mix': 'Mélange',
  },
)
