import type { Lang, MessageKey } from '../i18n/index'

/**
 * How long the output protection silenced (#667), from its count of stereo frames at `sampleRate`: a message
 * key and its number, already formatted for `lang` ("under 1 ms", whole milliseconds below one second, then
 * seconds with one decimal). A rate that is not usable counts as 48 kHz.
 */
export function silencedDuration(frames: number, sampleRate: number, lang: Lang): { key: MessageKey; n: string } {
  const rate = Number.isFinite(sampleRate) && sampleRate > 0 ? sampleRate : 48_000
  const ms = (Math.max(0, frames) * 1000) / rate
  if (ms < 1) return { key: 'engine.durationUnderMs', n: '' }
  if (Math.round(ms) < 1000) return { key: 'engine.durationMs', n: String(Math.round(ms)) }
  return { key: 'engine.durationS', n: new Intl.NumberFormat(lang, { maximumFractionDigits: 1 }).format(ms / 1000) }
}
