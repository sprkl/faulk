// Importing audio files into an audio track from the page (AU-1): `clip.choose_audio` opens the file
// dialog (from a click only), each chosen file goes through the store's pipeline (its bytes into
// `blobs/`, its decoded audio checked and cached), the metadata decoding gave is staged in the core,
// and `clip.import_audio` makes the clip: one history entry per file. An agent's file from a link
// (`clip.import_audio_url`, #664) is downloaded once the user allowed it (`importUrl`), then imported the
// same way. In Boost, with the project's folder open in the helper (#778), the helper imports instead: the
// files the user picks in the helper's own dialog (`clip.choose_audio` from a click), and an agent's file of
// the project's folder (`clip.import_audio_file`, `importFolderFile`); the page stages what the helper
// decoded and makes the clip the same way. What the Clips panel shows: the file being downloaded or
// imported and how far it is, the failure of the last one, a notice.
import type { AudioMedia, Command, ImportSource } from '../../generated/protocol'
import { checkOpenUrl, type CheckedUrl } from '../../storage/open-url'
import { StoreError } from '../../storage/types'
import { ImportError } from './abi'
import type { Downloaded } from './from-url'
import { chooseWithHelper, type HelperImportPort } from './helper-import'
import type { ImportedMedia } from './pipeline'

/** The browser and the page's project, behind seams so the controller is tested without them. */
export interface AudioImportDeps {
  /** Whether a dialog may open now (the browser wants a click behind it). */
  canAsk(): boolean
  /** Opens the file dialog; the files chosen, none when it was closed. Call straight from a click. */
  pick(): Promise<File[]>
  /** The store's import (`ProjectStorage.importAudio`). */
  importFile(file: File, onProgress: (fraction: number) => void, signal: AbortSignal): Promise<ImportedMedia>
  /** Stages the file's metadata in the core (`ProjectCore.stageMedia`); false when there is no core. */
  stage(media: AudioMedia): boolean
  /** Runs a command through the dispatcher. */
  run(command: Command): boolean
  /** The page's own host (`location.hostname`): plain `http` is fetched only to this computer from a page served by it. */
  pageHostname(): string
  /** Downloads the file at `url` (`downloadAudio`); `onProgress` gets the bytes received and the length announced. */
  download(url: URL, name: string, onProgress: (received: number, announced: number | null) => void, signal: AbortSignal): Promise<Downloaded>
  /** The helper's import (#778): Boost, a helper that imports, and the project's folder open in it; `null` otherwise. */
  helper(): HelperImportPort | null
  /** The helper's import of `source`, in the storage's queue (`ProjectStorage.importWithHelper` around `runHelperImport`). */
  importWithHelper(port: HelperImportPort, source: ImportSource, onProgress: (fraction: number) => void, signal: AbortSignal): Promise<ImportedMedia>
}

/** A file being downloaded from a link (#664), before its import. */
export interface DownloadProgress {
  track: string
  name: string
  host: string
  received: number
  /** The length the host announced, `null` when it announced none. */
  total: number | null
}

/** How an agent's file from a link ended (`importUrl`): its SHA-256, or why nothing was imported (the agent's error code). */
export type UrlImport = { status: 'ok'; hash: string } | { status: 'refused'; code: string; message: string }

export interface ImportProgress {
  track: string
  name: string
  /** The file's place among those chosen together (from 1), and their count. */
  index: number
  count: number
  /** From 0 to 1. */
  fraction: number
}

/** Why the last import failed: an `ImportError` code (the file) or a `StoreError` code (the folder). */
export interface ImportFailure {
  track: string
  name: string
  code: string
  message: string
}

/** The extensions the dialog offers (what the pipeline decodes; Opus in Ogg and WebM since #609). */
export const AUDIO_ACCEPT = 'audio/*,.wav,.wave,.aif,.aiff,.aifc,.caf,.flac,.mp3,.mp2,.m4a,.mp4,.aac,.alac,.ogg,.oga,.opus,.webm,.weba'

export const importCommand = (track: string, media: string): Command => ({ type: 'clip.import_audio', track, media, name: null })

/**
 * Where an imported file's clip goes (AU-6, a drop on a lane or a cell): runs the commands that make the clip (and place
 * it) once the file is read and its metadata staged; `index` is the file's place among those imported together. True
 * when the clip was made. Without one, the clip goes into the track's pool.
 */
export type ImportPlacer = (media: AudioMedia, index: number) => boolean

export class AudioImport {
  /** The import under way, or `null`. */
  running = $state.raw<ImportProgress | null>(null)
  /** The failure of the last import, until dismissed or the next one starts. */
  failure = $state.raw<ImportFailure | null>(null)
  /** A dialog asked for without a click (an agent's `clip.choose_audio`). */
  needsClick = $state(false)
  /** The files being downloaded from a link, in the order they were allowed. */
  downloads = $state.raw<DownloadProgress[]>([])
  private abort: AbortController | null = null
  /** The stops of the files from links, from their download to their import: a cancel stops them too. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- never read by the interface
  private linkAborts = new Set<AbortController>()
  /** The links being downloaded or imported: a second call for one of them is refused (`import.busy`). */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- never read by the interface
  private links = new Set<string>()
  private queue: Promise<unknown> = Promise.resolve()

  constructor(private readonly deps: AudioImportDeps) {}

  /** `clip.choose_audio`: the dialog, then each file chosen. Call straight from the click (the command runs in it). */
  choose(track: string) {
    if (!this.deps.canAsk()) {
      this.needsClick = true
      return
    }
    this.needsClick = false
    // In Boost with the folder open in the helper (#778): the helper's own dialog, the helper importing each file.
    const helper = this.deps.helper()
    if (helper) {
      const picking = chooseWithHelper(helper)
      void this.enqueue(async () => {
        let files
        try {
          files = await picking
        } catch (error) {
          // A helper gone with its dialog open: the page tells of the lost helper already, nothing was being imported.
          const code = error instanceof ImportError ? error.code : 'io'
          if (code !== 'helper_lost') this.failure = { track, name: '', code, message: String(error) }
          return []
        }
        const imports = files.map((file) => ({
          name: file.name,
          read: (onProgress: (fraction: number) => void, signal: AbortSignal) =>
            this.deps.importWithHelper(helper, { kind: 'picked', token: file.token }, onProgress, signal),
        }))
        return this.runImports(track, imports)
      })
      return
    }
    // Opened now, inside the click; the files are imported in turn, after any import still running.
    const picking = this.deps.pick()
    void this.enqueue(async () => this.runFiles(track, await picking))
  }

  /**
   * An agent's file of the project's folder (`clip.import_audio_file`, #778): imported by the helper in turn, like a picked
   * file, its clip made by `place`. `path` is relative to the folder. `signal` is the agent's call; `cancel` stops it too.
   * `stillWanted` as for `importUrl`. Refused `import.no_folder` without the helper's folder; `import.busy` while the same
   * path is being imported.
   */
  async importFolderFile(track: string, path: string, place: ImportPlacer, signal?: AbortSignal, stillWanted: () => boolean = () => true): Promise<UrlImport> {
    const key = `folder:${path}`
    if (this.links.has(key))
      return { status: 'refused', code: 'import.busy', message: 'this file is already being imported: wait for its answer, then check track.get' }
    const helper = this.deps.helper()
    if (!helper)
      return {
        status: 'refused',
        code: 'import.no_folder',
        message: 'the helper has no folder open for this project (Boost mode, the folder shown to the helper): nothing was read',
      }
    this.links.add(key)
    const stop = new AbortController()
    const onEnd = () => stop.abort()
    signal?.addEventListener('abort', onEnd, { once: true })
    if (signal?.aborted) stop.abort()
    this.linkAborts.add(stop)
    const name = path.split('/').at(-1) ?? path
    try {
      return await this.enqueue(async () => {
        if (stop.signal.aborted) return { status: 'refused', code: 'import.cancelled', message: 'the import was cancelled: nothing changed' } as const
        if (!stillWanted()) {
          return { status: 'refused', code: 'import.project_changed', message: 'another project was opened meanwhile: nothing changed' } as const
        }
        // The helper of the turn: the session may have ended while the file waited.
        const now = this.deps.helper()
        if (!now)
          return { status: 'refused', code: 'import.no_folder', message: 'the helper has no folder open for this project any more: nothing was read' } as const
        const abort = new AbortController()
        const onStop = () => abort.abort()
        stop.signal.addEventListener('abort', onStop, { once: true })
        this.abort = abort
        this.failure = null
        try {
          const read = (onProgress: (fraction: number) => void, s: AbortSignal) => this.deps.importWithHelper(now, { kind: 'folder', path }, onProgress, s)
          const one = await this.importOne(track, name, read, 0, 1, abort.signal, place)
          if (one.status === 'ok') return one
          if (one.status === 'cancelled') return { status: 'refused', code: 'import.cancelled', message: 'the import was cancelled: nothing changed' } as const
          return { status: 'refused', code: `import.${one.code}`, message: `${one.message}: nothing changed` } as const
        } finally {
          stop.signal.removeEventListener('abort', onStop)
          this.running = null
          if (this.abort === abort) this.abort = null
        }
      })
    } finally {
      signal?.removeEventListener('abort', onEnd)
      this.linkAborts.delete(stop)
      this.links.delete(key)
    }
  }

  /** Whether the file of the folder at `path` is being imported now (#778). */
  importingFile(path: string): boolean {
    return this.links.has(`folder:${path}`)
  }

  /**
   * Imports `files` into `track`, one after the other; returns the SHA-256 of each file imported. `place` chooses where
   * each clip goes (the pool by default).
   */
  importFiles(track: string, files: File[], place?: ImportPlacer): Promise<string[]> {
    return this.enqueue(() => this.runFiles(track, files, place))
  }

  /**
   * An agent's file from a link (#664), once the user allowed it: downloaded at once (not behind the imports under
   * way, so that a long batch of the user's does not hold it), then imported in turn like a picked file, its clip
   * made by `place` (the dispatcher runs `clip.import_audio` as the agent). `signal` is the agent's call: when it
   * ends, nothing more is done. `cancel` stops it too. `stillWanted` is asked when the import's turn comes: false (the
   * project the user allowed it for was replaced meanwhile) imports nothing (`import.project_changed`).
   */
  async importUrl(
    track: string,
    url: URL,
    host: string,
    name: string,
    place: ImportPlacer,
    signal?: AbortSignal,
    stillWanted: () => boolean = () => true,
  ): Promise<UrlImport> {
    const link = url.href
    if (this.links.has(link))
      return { status: 'refused', code: 'import.busy', message: 'this link is already being imported: wait for its answer, then check track.get' }
    this.links.add(link)
    const stop = new AbortController()
    const onEnd = () => stop.abort()
    signal?.addEventListener('abort', onEnd, { once: true })
    if (signal?.aborted) stop.abort()
    this.linkAborts.add(stop)
    const entry: DownloadProgress = { track, name, host, received: 0, total: null }
    const progress = (next: DownloadProgress | null) => {
      this.downloads = this.downloads.flatMap((d) => (d === current ? (next ? [next] : []) : [d]))
      if (next) current = next
    }
    let current = entry
    this.downloads = [...this.downloads, entry]
    try {
      let downloaded: Downloaded
      try {
        // Shown about every percent (or every 256 KiB without a length), not at each piece the network hands over.
        downloaded = await this.deps.download(
          url,
          name,
          (received, total) => {
            const step = total ? Math.max(total / 100, 1) : 256 << 10
            if (received - current.received >= step || (total !== null && received >= total)) progress({ ...current, received, total })
          },
          stop.signal,
        )
      } catch (error) {
        // A download that throws (memory for the file) is a download that failed, told as such.
        downloaded = { status: 'unreachable', detail: error instanceof Error ? error.message : String(error) }
      } finally {
        progress(null)
      }
      if (downloaded.status !== 'ok') {
        if (downloaded.status === 'cancelled') return { status: 'refused', code: 'import.cancelled', message: 'the download was cancelled: nothing changed' }
        if (downloaded.status === 'tooBig') {
          this.failure = { track, name, code: 'download_too_big', message: '' }
          return { status: 'refused', code: 'download.too_big', message: 'the file is bigger than the page downloads: nothing changed' }
        }
        this.failure = { track, name, code: 'download_unreachable', message: downloaded.detail }
        return { status: 'refused', code: 'download.unreachable', message: `the file could not be downloaded (${downloaded.detail}): nothing changed` }
      }
      return await this.enqueue(async () => {
        if (stop.signal.aborted) return { status: 'refused', code: 'import.cancelled', message: 'the import was cancelled: nothing changed' } as const
        if (!stillWanted()) {
          return {
            status: 'refused',
            code: 'import.project_changed',
            message: 'another project was opened since the user allowed it: nothing changed',
          } as const
        }
        const abort = new AbortController()
        const onStop = () => abort.abort()
        stop.signal.addEventListener('abort', onStop, { once: true })
        this.abort = abort
        this.failure = null
        try {
          const file = downloaded.file
          const read = (onProgress: (fraction: number) => void, s: AbortSignal) => this.deps.importFile(file, onProgress, s)
          const one = await this.importOne(track, file.name, read, 0, 1, abort.signal, place)
          if (one.status === 'ok') return one
          if (one.status === 'cancelled') return { status: 'refused', code: 'import.cancelled', message: 'the import was cancelled: nothing changed' } as const
          return { status: 'refused', code: `import.${one.code}`, message: `${one.message}: nothing changed` } as const
        } finally {
          stop.signal.removeEventListener('abort', onStop)
          this.running = null
          if (this.abort === abort) this.abort = null
        }
      })
    } finally {
      signal?.removeEventListener('abort', onEnd)
      this.linkAborts.delete(stop)
      this.links.delete(link)
    }
  }

  /** Whether the file at `url` is being downloaded or imported now (a second call for it is refused, `import.busy`). */
  importing(url: URL): boolean {
    return this.links.has(url.href)
  }

  /** Whether the page fetches `raw` (`checkOpenUrl`, with the page's own host). */
  checkLink(raw: string): CheckedUrl {
    return checkOpenUrl(raw, this.deps.pageHostname())
  }

  /** Stops the import under way (the files after it are not imported either) and the files from links, downloading or waiting their turn. */
  cancel() {
    this.abort?.abort()
    for (const stop of this.linkAborts) stop.abort()
  }

  dismiss() {
    this.failure = null
    this.needsClick = false
  }

  private enqueue<T>(task: () => Promise<T>): Promise<T> {
    const run = this.queue.then(task, task)
    this.queue = run.catch(() => undefined)
    return run
  }

  private runFiles(track: string, files: File[], place?: ImportPlacer): Promise<string[]> {
    const imports = files.map((file) => ({
      name: file.name,
      read: (onProgress: (fraction: number) => void, signal: AbortSignal) => this.deps.importFile(file, onProgress, signal),
    }))
    return this.runImports(track, imports, place)
  }

  /** Imports files in turn (each its name and its reader: the store's import, or the helper's). */
  private async runImports(track: string, files: { name: string; read: MediaReader }[], place?: ImportPlacer): Promise<string[]> {
    const imported: string[] = []
    if (files.length === 0) return imported
    this.failure = null
    const abort = new AbortController()
    this.abort = abort
    try {
      for (const [i, file] of files.entries()) {
        if (abort.signal.aborted) break
        const one = await this.importOne(track, file.name, file.read, i, files.length, abort.signal, place)
        if (one.status === 'cancelled') break
        // A failed file is reported (`failure`); the next files are still imported: one damaged file does not stop the others.
        if (one.status === 'ok') imported.push(one.hash)
      }
    } finally {
      this.running = null
      if (this.abort === abort) this.abort = null
    }
    return imported
  }

  /**
   * Imports one file named `name` (the `index`-th of `count`): read by `read` (the store or the helper), staged, made a
   * clip. A failure is also `failure`. A read that ends after a stop (the helper's last word, its blob kept) is cancelled.
   */
  private async importOne(
    track: string,
    name: string,
    read: MediaReader,
    index: number,
    count: number,
    signal: AbortSignal,
    place?: ImportPlacer,
  ): Promise<{ status: 'ok'; hash: string } | { status: 'cancelled' } | { status: 'failed'; code: string; message: string }> {
    this.running = { track, name, index: index + 1, count, fraction: 0 }
    try {
      const media = await read((fraction) => {
        if (this.running?.name === name) this.running = { ...this.running, fraction }
      }, signal)
      if (signal.aborted) throw new ImportError('cancelled', 'the import was cancelled')
      const staged: AudioMedia = { hash: media.hash, sample_rate: media.sample_rate, channels: media.channels, frames: media.frames, name: media.name }
      if (!this.deps.stage(staged)) throw new ImportError('unavailable', 'the project is not ready yet')
      const made = place ? place(staged, index) : this.deps.run(importCommand(track, media.hash))
      if (!made) throw new ImportError('store', 'the clip could not be made')
      return { status: 'ok', hash: media.hash }
    } catch (error) {
      if (error instanceof ImportError && error.code === 'cancelled') return { status: 'cancelled' }
      const code = error instanceof ImportError || error instanceof StoreError ? error.code : 'io'
      const message = error instanceof Error ? error.message : String(error)
      if (!(error instanceof ImportError || error instanceof StoreError)) console.error(error)
      this.failure = { track, name, code, message }
      return { status: 'failed', code, message }
    }
  }
}

/** What reads one file into the project's folder: its media once in, progress from 0 to 1. */
type MediaReader = (onProgress: (fraction: number) => void, signal: AbortSignal) => Promise<ImportedMedia>

/** The file dialog: an `<input type="file">`, which every browser has and a check can drive. */
export function browserAudioPicker(): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.multiple = true
    input.accept = AUDIO_ACCEPT
    input.style.display = 'none'
    input.dataset.testid = 'audio-file-input'
    const done = (files: File[]) => {
      input.remove()
      resolve(files)
    }
    input.addEventListener('change', () => done([...(input.files ?? [])]), { once: true })
    input.addEventListener('cancel', () => done([]), { once: true })
    document.body.append(input)
    input.click()
  })
}

/** Whether a dialog may open now: the page has a click behind it (`navigator.userActivation`). */
export function browserCanAsk(): boolean {
  const activation = (navigator as Navigator & { userActivation?: { isActive: boolean } }).userActivation
  return activation ? activation.isActive : true
}
