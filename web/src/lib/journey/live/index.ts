/**
 * The Journey page's players, loaded by the page's inline bootstrap (`journeyBootScript`). The page
 * is complete without it: each entry prerenders a native `<audio controls>`; this module swaps it
 * for the page's own controls (play and pause, a seek slider, the elapsed time), keeps one track
 * playing at a time, and on the first play loads the visualiser (`viz.ts`) unless the reader asked
 * for reduced motion.
 *
 * DOM contract, per player (`Player.svelte`):
 * - `[data-fk-player]`: the root, which gets `data-state` (`idle`, `playing`, `paused`, `error`);
 * - `audio[data-fk-audio]`: the native element (its `controls` are removed here);
 * - `[data-fk-controls]`: the page's controls, `hidden` until this module runs;
 * - `button[data-fk-play]` with `data-label-play` and `data-label-pause`;
 * - `input[type=range][data-fk-seek]` with `data-valuetext` (`{elapsed}`, `{total}`);
 * - `[data-fk-elapsed]`, `[data-fk-total]`, `[data-fk-error]` (hidden until the audio fails);
 * - the cover, `[data-fk-stage][data-foci]`, found by the root's `data-fk-stage-id`.
 */

import { clock, fill, lengthOf, seekByKey } from './time'
import type { Viz } from './viz'

const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches

let context: AudioContext | undefined
/** One AudioContext for every player, made on the first play. */
function sharedContext(): AudioContext {
  context ??= new AudioContext()
  return context
}

class Player {
  readonly audio: HTMLAudioElement
  private readonly button: HTMLButtonElement
  private readonly seek: HTMLInputElement
  private readonly elapsed: HTMLElement
  private readonly total: HTMLElement
  private readonly error: HTMLElement | null
  private readonly stage: HTMLElement | null
  private readonly stated: number
  private dragging = false
  private viz: Viz | undefined
  private vizFailed = false

  constructor(
    readonly root: HTMLElement,
    private readonly others: () => Player[],
  ) {
    this.audio = root.querySelector('audio[data-fk-audio]')!
    this.button = root.querySelector('button[data-fk-play]')!
    this.seek = root.querySelector('input[data-fk-seek]')!
    this.elapsed = root.querySelector('[data-fk-elapsed]')!
    this.total = root.querySelector('[data-fk-total]')!
    this.error = root.querySelector('[data-fk-error]')
    this.stage = root.dataset.fkStageId ? document.getElementById(root.dataset.fkStageId) : null
    this.stated = Number(this.seek.max) || 0
    if (!this.audio || !this.button || !this.seek || !this.elapsed || !this.total) throw new Error('incomplete player')

    this.audio.controls = false
    this.audio.classList.add('is-enhanced')
    root.querySelector<HTMLElement>('[data-fk-controls]')!.hidden = false

    this.button.addEventListener('click', () => this.toggle())
    this.seek.addEventListener('input', () => {
      this.dragging = true
      this.show(Number(this.seek.value))
    })
    this.seek.addEventListener('change', () => {
      this.dragging = false
      this.jump(Number(this.seek.value))
    })
    this.seek.addEventListener('keydown', (e) => {
      const to = seekByKey(e.key, this.audio.currentTime, this.length())
      if (to === undefined) return
      e.preventDefault()
      this.jump(to)
    })
    const a = this.audio
    a.addEventListener('play', () => this.onPlay())
    a.addEventListener('pause', () => this.onStop())
    a.addEventListener('ended', () => this.onStop())
    a.addEventListener('timeupdate', () => {
      if (!this.dragging) this.show(a.currentTime)
    })
    a.addEventListener('durationchange', () => this.show(a.currentTime))
    a.addEventListener('error', () => this.fail())
    this.show(0)
  }

  private length(): number {
    return lengthOf(this.audio.duration, this.stated)
  }

  /** Puts `seconds` on the slider, the time text and the slider's spoken value. */
  private show(seconds: number): void {
    const total = this.length()
    const now = Math.min(total, Math.max(0, seconds))
    this.seek.max = String(total)
    if (!this.dragging) this.seek.value = String(now)
    this.seek.style.setProperty('--played', `${total > 0 ? (now / total) * 100 : 0}%`)
    this.elapsed.textContent = clock(now)
    this.total.textContent = clock(total)
    this.seek.setAttribute('aria-valuetext', fill(this.seek.dataset.valuetext ?? '{elapsed}', { elapsed: clock(now), total: clock(total) }))
  }

  private jump(seconds: number): void {
    if (!Number.isFinite(seconds)) return
    this.audio.currentTime = seconds
    this.show(seconds)
  }

  toggle(): void {
    if (this.root.dataset.state === 'error') return
    if (this.audio.paused) {
      for (const other of this.others()) if (other !== this) other.audio.pause()
      this.startViz()
      this.audio.play().catch((err: unknown) => {
        // A pause or a new source interrupting play() is not a failure.
        if (!(err instanceof DOMException && err.name === 'AbortError')) this.fail()
      })
    } else {
      this.audio.pause()
    }
  }

  private label(playing: boolean): void {
    this.button.setAttribute('aria-label', (playing ? this.button.dataset.labelPause : this.button.dataset.labelPlay) ?? '')
  }

  private onPlay(): void {
    // A play started elsewhere (media keys) still keeps the others quiet.
    for (const other of this.others()) if (other !== this && !other.audio.paused) other.audio.pause()
    this.root.dataset.state = 'playing'
    this.label(true)
    this.viz?.start()
  }

  private onStop(): void {
    if (this.root.dataset.state !== 'error') this.root.dataset.state = this.audio.ended ? 'idle' : 'paused'
    this.label(false)
    this.viz?.stop()
  }

  private fail(): void {
    this.root.dataset.state = 'error'
    this.label(false)
    this.viz?.stop()
    this.button.disabled = true
    this.seek.disabled = true
    if (this.error) this.error.hidden = false
  }

  /**
   * Loads the visualiser on the first play. The AudioContext is made here, inside the click, so
   * the browser lets it start; the audio is routed through it only once it runs (a suspended
   * context would silence the track).
   */
  private startViz(): void {
    if (this.viz || this.vizFailed || !this.stage || reduced() || !window.AudioContext) return
    this.vizFailed = true
    let context: AudioContext
    try {
      context = sharedContext()
    } catch {
      return
    }
    Promise.all([import('./viz'), context.resume()])
      .then(([{ createViz }]) => {
        if (reduced() || context.state !== 'running') return
        this.viz = createViz(this.stage!, this.audio, context)
        this.vizFailed = false
        if (!this.audio.paused) this.viz.start()
      })
      .catch(() => {
        // Plays on without the visualiser.
      })
  }

  /** Reduced motion was switched on: drop the visualiser (the audio keeps playing). */
  calm(): void {
    this.viz?.destroy()
    this.viz = undefined
  }
}

export function mount(root: ParentNode = document): () => void {
  const players: Player[] = []
  for (const el of root.querySelectorAll<HTMLElement>('[data-fk-player]')) {
    try {
      players.push(new Player(el, () => players))
    } catch {
      // That entry keeps its native audio element.
    }
  }
  const media = matchMedia('(prefers-reduced-motion: reduce)')
  const onReduced = () => {
    if (media.matches) for (const p of players) p.calm()
  }
  media.addEventListener('change', onReduced)
  document.documentElement.dataset.fkPlayers = String(players.length)
  return () => media.removeEventListener('change', onReduced)
}
