/**
 * Event clocks of the percussive objects: when a kick hits, when an echo drops. Pure, so the
 * limits (how often something can change) are testable.
 */

/** WCAG 2.3.1: nothing changes more than three times per second; a hit every 340 ms at most. */
export const MIN_HIT_INTERVAL = 0.34

/**
 * The kick's hits: one every `period` seconds on its own, plus the hits the pointer triggers,
 * never closer than `MIN_HIT_INTERVAL`.
 */
export class HitClock {
  private last = -1e9
  private nextAuto: number

  constructor(
    readonly period = 1.2,
    phase = 0,
  ) {
    this.nextAuto = phase
  }

  /** Advances to `t`; returns true when a hit fires at this step. The beat never drifts. */
  tick(t: number): boolean {
    if (t < this.nextAuto) return false
    const due = this.nextAuto
    this.nextAuto = due + this.period * (Math.floor((t - due) / this.period) + 1)
    return this.fire(t, false)
  }

  /** A hit asked by the pointer; refused inside the minimum interval. */
  hit(t: number): boolean {
    return this.fire(t, true)
  }

  /** Seconds since the last hit (large when none happened yet). */
  since(t: number): number {
    return t - this.last
  }

  private fire(t: number, realign: boolean): boolean {
    if (t - this.last < MIN_HIT_INTERVAL) return false
    this.last = t
    // The autonomous beat realigns on a manual hit, so the rhythm stays even.
    if (realign) this.nextAuto = t + this.period
    return true
  }
}

/** A ripple source: position (stage fractions), start time and amplitude. */
export interface Drop {
  x: number
  y: number
  t0: number
  amp: number
}

/** The echo's drops, a ring buffer packed for a `vec4[]` uniform (a plain array, for OGL). */
export class DropRing {
  readonly data: number[]
  private next = 0
  private lastAuto = -1e9
  private lastPointer = -1e9

  constructor(
    readonly capacity = 12,
    readonly period = 0.9,
    readonly pointerInterval = 0.14,
  ) {
    this.data = new Array<number>(capacity * 4).fill(0)
    // Long-dead drops: the shader ignores anything older than a few seconds.
    for (let i = 0; i < capacity; i++) this.data[i * 4 + 2] = -1e3
  }

  /** The source at the centre, every `period` seconds. */
  tick(t: number): boolean {
    if (t - this.lastAuto < this.period) return false
    this.lastAuto = t
    this.add({ x: 0.5, y: 0.5, t0: t, amp: 1 })
    return true
  }

  /** A drop under a moving pointer, rate-limited; `speed` in stage fractions per second. */
  pointer(t: number, x: number, y: number, speed: number): boolean {
    if (t - this.lastPointer < this.pointerInterval || speed < 0.05) return false
    this.lastPointer = t
    this.add({ x, y, t0: t, amp: Math.min(1, 0.35 + speed * 0.4) })
    return true
  }

  add(drop: Drop): void {
    const i = this.next * 4
    this.data[i] = drop.x
    this.data[i + 1] = drop.y
    this.data[i + 2] = drop.t0
    this.data[i + 3] = drop.amp
    this.next = (this.next + 1) % this.capacity
  }

  /** Drops alive at `t` (younger than `maxAge`). */
  alive(t: number, maxAge = 4): number {
    let n = 0
    for (let i = 0; i < this.capacity; i++) if (t - this.data[i * 4 + 2] < maxAge) n++
    return n
  }
}
