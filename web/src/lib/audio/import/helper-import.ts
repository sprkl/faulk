// The native audio import in Boost (#778): the helper reads a file of the project's folder (or one the user picked in its
// own file dialog), decodes it with the page's decoder and writes its blob and caches into the folder; the page then makes
// the clip with its commands, as for its own import. Nothing of the file crosses the link: the page sends the job
// (`import.start`) and gets the media's metadata back (`import.done`).
import type { HelperImportMessage } from '../../engine/engine'
import { IMPORT_ERROR_CODES, IMPORT_PROGRESS_MS, type ImportSource, type PickedFile } from '../../generated/protocol'
import { ImportError, type ImportErrorCode } from './abi'
import { mediaName, type ImportedMedia } from './pipeline'

/** `import.start` without its `type`. */
export interface HelperImportStart {
  id: number
  project: string
  source: ImportSource
}

/** The helper's import, as the page's import drives it (the dispatcher implements it on the Boost session). */
export interface HelperImportPort {
  /** The project the helper has its folder open for. */
  project: string
  choose(id: number): void
  start(start: HelperImportStart): void
  cancel(id: number): void
  /** Tells `listener` each message of the helper's imports, and `null` once the helper session ended. Returns the unsubscribe. */
  listen(listener: (message: HelperImportMessage | null) => void): () => void
}

/** What the helper's import wrote, for the page to record. */
export interface HelperImported {
  media: ImportedMedia
  /** The helper wrote `blobs/<hash>` (the folder did not hold it). */
  created: boolean
  /** Its waveform peaks were written. */
  peaks: boolean
}

/**
 * How long the page waits for a word of the helper during its import before it takes the helper for gone: the helper says
 * how far it is every `IMPORT_PROGRESS_MS` while it works, whatever step it is in.
 */
export const HELPER_IMPORT_SILENCE_MS = 300 * IMPORT_PROGRESS_MS

/** The page's code of an `import.failed` code: `import.` dropped (`import.too_large` is `too_large`). */
export function failureCode(code: string): ImportErrorCode {
  const known: string[] = ['cancelled', 'no_folder', 'path', 'unreadable', 'write', 'folder_changed', 'busy', 'invalid', 'dialog_unavailable']
  const decoder: string[] = ['unsupported_format', 'unsupported_codec', 'corrupt', 'empty', 'bad_sample_rate', 'bad_channels', 'too_long', 'too_large', 'io']
  const bare = code.startsWith(IMPORT_ERROR_CODES.decode_prefix) ? code.slice(IMPORT_ERROR_CODES.decode_prefix.length) : code
  if (known.includes(bare) || decoder.includes(bare)) return bare as ImportErrorCode
  return 'io'
}

let nextId = 1

/** Timers, behind a seam for the tests. */
export interface Timers {
  set(f: () => void, ms: number): unknown
  clear(handle: unknown): void
}

const realTimers: Timers = {
  set: (f, ms) => setTimeout(f, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
}

/**
 * Runs one import by the helper to its last word. Resolves with what it wrote; rejects with an `ImportError` (the helper's
 * refusal, `helper_lost` when the session ended or the helper stayed silent for `HELPER_IMPORT_SILENCE_MS`). A stop
 * (`signal`) asks the helper to cancel but still waits for its last word, so that a blob it put in place just before is
 * still known to the page: it then resolves (the caller checks `signal`) or rejects `cancelled`.
 */
export function runHelperImport(
  port: HelperImportPort,
  source: ImportSource,
  onProgress: (fraction: number) => void,
  signal: AbortSignal,
  timers: Timers = realTimers,
): Promise<HelperImported> {
  const id = nextId++
  return new Promise<HelperImported>((resolve, reject) => {
    let silence: unknown = null
    let settled = false
    const finish = (outcome: () => void) => {
      if (settled) return
      settled = true
      if (silence !== null) timers.clear(silence)
      signal.removeEventListener('abort', onAbort)
      unlisten()
      outcome()
    }
    const watch = () => {
      if (silence !== null) timers.clear(silence)
      silence = timers.set(() => {
        port.cancel(id)
        finish(() => reject(new ImportError('helper_lost', 'the helper said nothing for too long during the import')))
      }, HELPER_IMPORT_SILENCE_MS)
    }
    const onAbort = () => port.cancel(id)
    const unlisten = port.listen((message) => {
      if (message === null) {
        finish(() => reject(new ImportError('helper_lost', 'the helper went away during the import')))
        return
      }
      if (message.type === 'import.chosen' || message.id !== id) return
      if (message.type === 'import.progress') {
        watch()
        onProgress(message.fraction)
      } else if (message.type === 'import.done') {
        const { hash, sample_rate, channels, frames, name, created, peaks } = message
        finish(() => resolve({ media: { hash, sample_rate, channels, frames, name: mediaName(name) }, created, peaks }))
      } else {
        finish(() => reject(new ImportError(failureCode(message.code), message.message)))
      }
    })
    signal.addEventListener('abort', onAbort, { once: true })
    watch()
    port.start({ id, project: port.project, source })
    if (signal.aborted) onAbort()
  })
}

/**
 * Shows the helper's file dialog (`import.choose`) and gives the files the user picked (none when the dialog was closed).
 * Rejects with an `ImportError` (`dialog_unavailable`, `busy`, `no_folder`, `helper_lost`). No silence watchdog: the user
 * takes the time they take.
 */
export function chooseWithHelper(port: HelperImportPort): Promise<PickedFile[]> {
  const id = nextId++
  return new Promise<PickedFile[]>((resolve, reject) => {
    const unlisten = port.listen((message) => {
      if (message === null) {
        unlisten()
        reject(new ImportError('helper_lost', 'the helper went away while its dialog was open'))
        return
      }
      if (message.id !== id) return
      if (message.type === 'import.chosen') {
        unlisten()
        resolve(message.files)
      } else if (message.type === 'import.failed') {
        unlisten()
        reject(new ImportError(failureCode(message.code), message.message))
      }
    })
    port.choose(id)
  })
}
