/**
 * The response curve of the equaliser Contour (#156), as its panel draws it: the same transfer
 * functions as the engine (`Contour::response_db` in `crates/engine/src/devices/contour.rs`), from
 * the values of its parameters. `response.fixture.json`, written by the engine's tests from that
 * function, holds both to the same numbers (`response.test.ts`).
 */

/** The bands, in the order of the signal: the keys' prefix and what each one does. */
export const BANDS = [
  { key: 'hp', shape: 'highpass' },
  { key: 'low', shape: 'lowshelf' },
  { key: 'bell1', shape: 'bell' },
  { key: 'bell2', shape: 'bell' },
  { key: 'bell3', shape: 'bell' },
  { key: 'high', shape: 'highshelf' },
  { key: 'lp', shape: 'lowpass' },
] as const

export type Band = (typeof BANDS)[number]
export type Shape = Band['shape']

/** A cut band (high-pass, low-pass): a slope and no gain. */
export const isCut = (shape: Shape): boolean => shape === 'highpass' || shape === 'lowpass'

export const FREQ_MIN = 20
export const FREQ_MAX = 20_000
export const GAIN_MAX = 24
export const Q_MIN = 0.1
export const Q_MAX = 18

/** The quality factors of a cut band's sections per slope (12, 24, 48 dB per octave), the highest last. */
const BUTTERWORTH: readonly (readonly number[])[] = [[Math.SQRT1_2], [0.5411961, 1.306563], [0.5097956, 0.6013449, 0.8999762, 2.5629154]]

/** The value of a parameter: what the device holds, else its default (`defaults`). */
export type Values = (key: string) => number

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/** `|H|^2` of one state-variable section at `w = tan(pi f / fs) / g`, with damping `k` and the mix `m`. */
function sectionPower(w: number, k: number, m0: number, m1: number, m2: number): number {
  const dr = 1 - w * w
  const di = k * w
  const nr = m0 * dr + m2
  const ni = m0 * di + m1 * w
  return (nr * nr + ni * ni) / (dr * dr + di * di)
}

/** The gain, in dB, of one band (on or off: the caller decides) at `hz`. */
export function bandDb(band: Band, values: Values, hz: number, sampleRate: number): number {
  const top = Math.max(0.45 * sampleRate, FREQ_MIN)
  const freq = clamp(values(`${band.key}_freq`), FREQ_MIN, top)
  const q = clamp(values(`${band.key}_q`), Q_MIN, Q_MAX)
  const t = Math.tan((Math.PI * clamp(hz, 0, 0.499 * sampleRate)) / sampleRate)
  const tf = Math.tan((Math.PI * freq) / sampleRate)
  if (isCut(band.shape)) {
    const slope = clamp(Math.round(values(`${band.key}_slope`)), 0, 2)
    const qs = BUTTERWORTH[slope]
    let power = 1
    qs.forEach((bw, i) => {
      const sq = i === qs.length - 1 ? (bw * q) / Math.SQRT1_2 : bw
      const k = 1 / sq
      power *= band.shape === 'highpass' ? sectionPower(t / tf, k, 1, -k, -1) : sectionPower(t / tf, k, 0, 0, 1)
    })
    return 10 * Math.log10(power)
  }
  const db = clamp(values(`${band.key}_gain`), -GAIN_MAX, GAIN_MAX)
  const a = db === 0 ? 1 : Math.pow(10, db / 40)
  let power: number
  if (band.shape === 'bell') {
    const k = 1 / (q * a)
    power = sectionPower(t / tf, k, 1, k * (a * a - 1), 0)
  } else if (band.shape === 'lowshelf') {
    const k = 1 / q
    power = sectionPower(t / (tf / Math.sqrt(a)), k, 1, k * (a - 1), a * a - 1)
  } else {
    const k = 1 / q
    power = sectionPower(t / (tf * Math.sqrt(a)), k, a * a, k * (1 - a) * a, 1 - a * a)
  }
  return 10 * Math.log10(power)
}

/** Whether a band is on. */
export const isOn = (band: Band, values: Values): boolean => values(`${band.key}_on`) >= 0.5

/** The gain, in dB, of the whole equaliser at `hz`: the bands that are on, then the output gain. */
export function responseDb(values: Values, hz: number, sampleRate: number): number {
  let db = values('output')
  for (const band of BANDS) if (isOn(band, values)) db += bandDb(band, values, hz, sampleRate)
  return db
}

/** Position (0 to 1) of a frequency on the panel's log axis, 20 Hz to 20 kHz. */
export const xOfHz = (hz: number): number => Math.log(clamp(hz, FREQ_MIN, FREQ_MAX) / FREQ_MIN) / Math.log(FREQ_MAX / FREQ_MIN)

/** The frequency at a position (0 to 1) of the log axis. */
export const hzOfX = (x: number): number => FREQ_MIN * Math.pow(FREQ_MAX / FREQ_MIN, clamp(x, 0, 1))

/** The frequencies the curve is drawn at: `n` points evenly spaced on the log axis. */
export const curveHz = (n: number): number[] => Array.from({ length: n }, (_, i) => hzOfX(i / (n - 1)))
