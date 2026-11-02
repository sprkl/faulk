/** How the Journey page writes times, keys and days, in English and French. */

import type { Lang } from '../i18n/lang'
import type { Mode, Tonic } from './types'

/** `m:ss` from seconds; 0:00 for anything that is not a finite positive number. */
export function clock(seconds: number): string {
  const s = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

const FR_TONIC: Record<string, string> = { C: 'Do', D: 'Ré', E: 'Mi', F: 'Fa', G: 'Sol', A: 'La', B: 'Si' }
const MODE: Record<Lang, Record<Mode, string>> = {
  en: { major: 'major', minor: 'minor', dorian: 'Dorian', mixolydian: 'Mixolydian', phrygian: 'Phrygian', lydian: 'Lydian' },
  fr: { major: 'majeur', minor: 'mineur', dorian: 'dorien', mixolydian: 'mixolydien', phrygian: 'phrygien', lydian: 'lydien' },
}

/** `A minor`, `La mineur`, `F# Dorian`, `Fa# dorien`. */
export function keyName(key: { tonic: Tonic; mode: Mode }, lang: Lang): string {
  const tonic = lang === 'fr' ? FR_TONIC[key.tonic[0]] + key.tonic.slice(1) : key.tonic
  return `${tonic} ${MODE[lang][key.mode]}`
}

const MONTHS: Record<Lang, string[]> = {
  en: ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'],
  fr: ['janvier', 'février', 'mars', 'avril', 'mai', 'juin', 'juillet', 'août', 'septembre', 'octobre', 'novembre', 'décembre'],
}

/** `2 November 2026`, `2 novembre 2026`: the same on the server and in every browser. */
export function dayName(date: string, lang: Lang): string {
  const [y, m, d] = date.split('-').map(Number)
  return `${d} ${MONTHS[lang][m - 1]} ${y}`
}

/** `November 2026`, `novembre 2026` from `2026-11`. */
export function monthName(month: string, lang: Lang): string {
  const [y, m] = month.split('-').map(Number)
  return `${MONTHS[lang][m - 1]} ${y}`
}

const SHORT: Record<Lang, string[]> = {
  en: ['JAN', 'FEB', 'MAR', 'APR', 'MAY', 'JUN', 'JUL', 'AUG', 'SEP', 'OCT', 'NOV', 'DEC'],
  fr: ['JANV.', 'FÉVR.', 'MARS', 'AVR.', 'MAI', 'JUIN', 'JUIL.', 'AOÛT', 'SEPT.', 'OCT.', 'NOV.', 'DÉC.'],
}

/** `02`, `NOV 2026`: the date rail's two lines (decorative: the full day is written next to them). */
export function dayParts(date: string, lang: Lang): { day: string; month: string } {
  const [y, m, d] = date.split('-')
  return { day: d, month: `${SHORT[lang][Number(m) - 1]} ${y}` }
}
