// The audio export in Boost (#319): what the page sends the helper, which renders the job and writes
// the file into the project's folder (`exports/`). The page still builds the job; only its bytes cross.
import { EXPORT_NAME } from '../generated/protocol'
import type { HelperExportMessage } from '../engine/engine'
import { cleanName } from './audio-export.svelte'

/** `export.start` without its `type`: the job's streams in standard base64. */
export interface HelperExportStart {
  id: number
  project: string
  name: string
  sample_rate: number
  bits: number
  body_frames: number
  setup: string
  start: string
  stop: string
}

/** The helper's export, as the export dialog drives it (the dispatcher implements it on the Boost session). */
export interface HelperExportPort {
  start(start: HelperExportStart): void
  cancel(id: number): void
  /** Tells `listener` each message of the helper's exports, and `null` once the helper session ended. Returns the unsubscribe. */
  listen(listener: (message: HelperExportMessage | null) => void): () => void
}

/**
 * The bytes of `view` (only its own bytes, wherever it sits in its buffer) in standard base64 (`+`, `/`,
 * padded), as the helper decodes them.
 */
export function toBase64(view: Uint8Array): string {
  const CHUNK = 0x8000
  const parts: string[] = []
  for (let at = 0; at < view.length; at += CHUNK) parts.push(String.fromCharCode(...view.subarray(at, at + CHUNK)))
  return btoa(parts.join(''))
}

const DEVICES: readonly string[] = EXPORT_NAME.devices

/**
 * The file name the helper takes for the dialog's name (the stem, without `.wav`): `cleanName`'s, cut to the
 * helper's byte limit on a whole character, and one that is not a Windows device name (`CON`, `nul.txt`), which
 * gets a `_` in front. Every name the dialog's field can hold gives a name the helper accepts.
 */
export function helperFileName(name: string): string {
  let cleaned = cleanName(name)
  const encoder = new TextEncoder()
  if (encoder.encode(cleaned).length > EXPORT_NAME.max_bytes) {
    let cut = ''
    for (const char of cleaned) {
      if (encoder.encode(cut + char).length > EXPORT_NAME.max_bytes) break
      cut += char
    }
    cleaned = cleanName(cut)
  }
  const stem = cleaned.split('.')[0]!.trimEnd().toLowerCase()
  if (DEVICES.includes(stem)) cleaned = `_${cleaned}`
  // `_` added to a name at the limit: cut again (a device name is short, so this keeps the `_`).
  while (encoder.encode(cleaned).length > EXPORT_NAME.max_bytes)
    cleaned = Array.from(cleaned)
      .slice(0, -1)
      .join('')
      .replace(/[\s.]+$/, '')
  return cleaned
}
