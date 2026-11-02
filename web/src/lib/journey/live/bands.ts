/**
 * What the visualiser reads from the analyser: one energy per colour focus, from log-spaced
 * frequency bands (the first focus follows the lows, the last the highs). Pure, tested.
 */

/**
 * Band `i` of `count` over `bins` FFT bins, as `[from, to)`: log-spaced from bin 1 (skipping DC) to
 * the top, each at least one bin wide.
 */
export function bandRange(i: number, count: number, bins: number): [number, number] {
  const lo = 1
  const hi = Math.max(lo + count, bins)
  const at = (k: number) => Math.round(lo * Math.pow(hi / lo, k / count))
  const from = Math.min(bins - 1, Math.max(lo, at(i)))
  const to = Math.min(bins, Math.max(from + 1, at(i + 1)))
  return [from, to]
}

/** Mean level (0..1) of each band of a byte spectrum, written into `out`. */
export function bandLevels(spectrum: ArrayLike<number>, out: Float32Array): Float32Array {
  const count = out.length
  for (let i = 0; i < count; i++) {
    const [from, to] = bandRange(i, count, spectrum.length)
    let sum = 0
    for (let k = from; k < to; k++) sum += spectrum[k]
    out[i] = to > from ? sum / (to - from) / 255 : 0
  }
  return out
}

/**
 * Follows `target` with a fast attack and a slow release, like a meter: `dt` in seconds. Never
 * leaves 0..1, whatever the inputs.
 */
export function follow(current: number, target: number, dt: number): number {
  const c = Number.isFinite(current) ? current : 0
  const t = Number.isFinite(target) ? Math.min(1, Math.max(0, target)) : 0
  const d = Number.isFinite(dt) && dt > 0 ? Math.min(dt, 0.1) : 0
  const rate = t > c ? 18 : 3.5
  return Math.min(1, Math.max(0, c + (t - c) * (1 - Math.exp(-rate * d))))
}
