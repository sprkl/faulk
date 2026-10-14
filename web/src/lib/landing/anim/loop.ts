import { frameDelta } from './gate'

/** A requestAnimationFrame loop with start and stop; `frame` gets the time and the clamped delta. */
export class Loop {
  running = false
  /** Animation time in seconds: advances only while running. */
  t = 0
  private raf = 0
  private last = 0

  constructor(private readonly frame: (t: number, dt: number) => void) {}

  start(): void {
    if (this.running) return
    this.running = true
    this.last = performance.now()
    this.raf = requestAnimationFrame(this.tick)
  }

  stop(): void {
    if (!this.running) return
    this.running = false
    cancelAnimationFrame(this.raf)
  }

  private readonly tick = (now: number) => {
    if (!this.running) return
    const dt = frameDelta(now, this.last)
    this.last = now
    this.t += dt
    this.frame(this.t, dt)
    if (this.running) this.raf = requestAnimationFrame(this.tick)
  }
}
