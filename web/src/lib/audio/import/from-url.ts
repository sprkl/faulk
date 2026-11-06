// An audio file an agent names by its link (#664, `clip.import_audio_url`): its name for the question and
// the clip, and its download into a `File` that then goes the way of a file the user picks (the store's
// pipeline decodes and checks its content; its extension and type are never trusted). The link was checked
// by the core already and again by the page (`checkOpenUrl`); the download is `streamPack`'s: no cookies,
// credentials or referrer, CORS, a redirect checked as the link, the size counted and cut, a stall given up.
import { MAX_AUDIO_URL_BYTES } from '../../generated/protocol'
import { lastSegmentOf, streamPack, type Streamed } from '../../storage/open-url'
import { mediaName } from './pipeline'

/**
 * What the question and the clip list may not show of a name the agent chose: separators, control characters,
 * the marks that reorder text (bidirectional controls) and quotation marks, which could close the name's quotes in
 * the question and make the rest of it read as the user's own words or another host.
 */
// eslint-disable-next-line no-control-regex
const UNSHOWN = /[\u0000-\u001f\u007f-\u009f/\\"'`\u00ab\u00bb\u2018-\u201f\u2039\u203a\u200e\u200f\u202a-\u202e\u2066-\u2069]/g

/** The longest name the question shows: a file's name, not a sentence. */
export const MAX_SHOWN_NAME = 80

/** The name of the file at `url`: the last part of its path, percent-decoded, as the question shows it; `audio` for none. */
export function audioNameOf(url: URL): string {
  const name = Array.from(mediaName(lastSegmentOf(url).replace(UNSHOWN, ' ').replace(/\s+/g, ' ')))
  const shown =
    name.length > MAX_SHOWN_NAME
      ? `${name
          .slice(0, MAX_SHOWN_NAME - 1)
          .join('')
          .trimEnd()}…`
      : name.join('')
  return shown || 'audio'
}

export type Downloaded = { status: 'ok'; file: File } | Exclude<Streamed, { status: 'ok' }>

export interface AudioDownloadOptions {
  fetch: typeof fetch
  pageHostname: string
  /** Stops the download (`cancelled`). */
  signal?: AbortSignal
  /** The bytes received, and the length the host announced (`null` when none). */
  onProgress?: (received: number, announced: number | null) => void
  /** For tests: the cap (`MAX_AUDIO_URL_BYTES`) and the stall (`STALL_MS`). */
  max?: number
  stallMs?: number
}

/** Downloads the file at `url` into memory, at most `MAX_AUDIO_URL_BYTES`, as a `File` named `name`. */
export async function downloadAudio(url: URL, name: string, options: AudioDownloadOptions): Promise<Downloaded> {
  const chunks: Uint8Array<ArrayBuffer>[] = []
  const streamed = await streamPack(url, {
    fetch: options.fetch,
    max: options.max ?? MAX_AUDIO_URL_BYTES,
    pageHostname: options.pageHostname,
    stallMs: options.stallMs,
    signal: options.signal,
    onProgress: options.onProgress,
    sink: { write: async (chunk) => void chunks.push(chunk as Uint8Array<ArrayBuffer>) },
  })
  if (streamed.status !== 'ok') return streamed
  // No type: the decoder reads the content, whatever the host said it was.
  return { status: 'ok', file: new File(chunks, name) }
}
