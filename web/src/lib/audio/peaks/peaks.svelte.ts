// The waveform peaks the page has read (AU-2), by media. A view asks for a media's peaks with `of`; the
// first ask reads `.faulk/cache/peaks/<sha256>.peaks` from the project's place (through the store's
// Worker, never on the main thread's time). A file that is absent or doubtful is first waited for (the
// opening's background work makes the caches a folder lacks), then asked for (the Worker decodes the
// blob), then read again: nothing blocks, and the view draws a placeholder meanwhile. Peaks are named by
// the blob, so they stay valid whatever place the project moves to; a media left without peaks is tried
// again after the next attach.
import { SvelteMap } from 'svelte/reactivity'
import type { MediaRef } from '../import/pipeline'
import { parsePeaks, type Peaks } from './format'

export interface PeaksSource {
  /** The peaks file of `hash` in the project's place, or `null` when it has none (or there is no place). */
  read(hash: string): Promise<Uint8Array | null>
  /** Resolves once the opening's background caches are made (or given up). */
  settled(): Promise<void>
  /** Has the peaks of `media` made; resolves when done or refused (never rejects). */
  make(media: MediaRef): Promise<void>
  /** Counts the attaches: a media left without peaks is tried again in a new one. */
  epoch(): number
}

export type PeaksEntry =
  | { state: 'loading'; peaks: null }
  | { state: 'ready'; peaks: Peaks }
  /** No peaks in this place for now: no file and none could be made (no place, the blob missing, a decode refused). */
  | { state: 'absent'; peaks: null; epoch: number }

const LOADING: PeaksEntry = { state: 'loading', peaks: null }

export class PeaksCache {
  private readonly entries = new SvelteMap<string, PeaksEntry>()
  /** The loads under way (not reactive: `of` reads it inside a derivation). */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- read inside derivations on purpose: never a dependency
  private readonly running = new Set<string>()

  constructor(private readonly source: PeaksSource) {}

  /**
   * The peaks of `media`, as far as they are known now: reactive, and safe to call inside a
   * derivation or a template (the load it may start runs after, never inside it).
   */
  of(media: MediaRef): PeaksEntry {
    const entry = this.entries.get(media.hash)
    const epoch = this.source.epoch()
    if (entry?.state === 'ready') return entry
    if (entry?.state === 'absent' && entry.epoch === epoch) return entry
    if (!this.running.has(media.hash)) {
      this.running.add(media.hash)
      queueMicrotask(() => void this.load(media, epoch))
    }
    return entry ?? LOADING
  }

  /** What is known of `hash` (tests, the dev hook). */
  peek(hash: string): PeaksEntry | undefined {
    return this.entries.get(hash)
  }

  /** Every load under way, settled (tests). */
  idle(): Promise<void> {
    return this.waiting.length === 0 ? Promise.resolve() : Promise.all(this.waiting).then(() => this.idle())
  }

  private waiting: Promise<void>[] = []

  private async read(media: MediaRef): Promise<Peaks | null> {
    try {
      const bytes = await this.source.read(media.hash)
      return bytes ? parsePeaks(bytes, media) : null
    } catch {
      return null
    }
  }

  private load(media: MediaRef, epoch: number): Promise<void> {
    const run = (async () => {
      try {
        let peaks = await this.read(media)
        if (!peaks) {
          // The opening may be making it now.
          await this.source.settled().catch(() => undefined)
          peaks = await this.read(media)
        }
        if (!peaks) {
          await this.source.make(media).catch(() => undefined)
          peaks = await this.read(media)
        }
        this.entries.set(media.hash, peaks ? { state: 'ready', peaks } : { state: 'absent', peaks: null, epoch })
      } finally {
        this.running.delete(media.hash)
        this.waiting = this.waiting.filter((p) => p !== run)
      }
    })()
    this.waiting.push(run)
    return run
  }
}
