/** The one pointer of the page (mouse, pen or finger), in viewport CSS px, with its velocity. */

export class Pointer {
  x = -1e4
  y = -1e4
  vx = 0
  vy = 0
  /** False until the first move, and after a finger lifts. */
  present = false
  down = false
  /** Time (performance.now, ms) of the last move. */
  lastMove = -1e9
  private sx = -1e4
  private sy = -1e4
  private st = 0
  private off: (() => void) | undefined

  attach(target: Window = window): void {
    const move = (e: PointerEvent) => {
      this.x = e.clientX
      this.y = e.clientY
      this.present = true
      this.lastMove = e.timeStamp
    }
    const down = (e: PointerEvent) => {
      move(e)
      this.down = true
    }
    const up = (e: PointerEvent) => {
      this.down = false
      if (e.pointerType === 'touch') this.present = false
    }
    const leave = () => {
      this.present = false
    }
    target.addEventListener('pointermove', move, { passive: true })
    target.addEventListener('pointerdown', down, { passive: true })
    target.addEventListener('pointerup', up, { passive: true })
    target.addEventListener('pointercancel', up, { passive: true })
    target.document.documentElement.addEventListener('pointerleave', leave)
    this.off = () => {
      target.removeEventListener('pointermove', move)
      target.removeEventListener('pointerdown', down)
      target.removeEventListener('pointerup', up)
      target.removeEventListener('pointercancel', up)
      target.document.documentElement.removeEventListener('pointerleave', leave)
    }
  }

  detach(): void {
    this.off?.()
    this.off = undefined
  }

  /** Once per frame: velocity from the last two frames, smoothed. */
  update(nowMs: number): void {
    const dt = (nowMs - this.st) / 1000
    if (dt > 0 && this.sx > -1e3) {
      const k = Math.min(1, dt * 14)
      this.vx += ((this.x - this.sx) / dt - this.vx) * k
      this.vy += ((this.y - this.sy) / dt - this.vy) * k
    }
    this.sx = this.x
    this.sy = this.y
    this.st = nowMs
  }
}

/**
 * Where a virtual pointer wanders when nobody touches the page: a slow Lissajous figure inside
 * the box, so the hero lives on screens without a pointer.
 */
export function idlePoint(t: number, box: { x: number; y: number; w: number; h: number }): { x: number; y: number } {
  return {
    x: box.x + box.w * (0.5 + 0.42 * Math.sin(t * 0.23)),
    y: box.y + box.h * (0.5 + 0.38 * Math.sin(t * 0.37 + 1.1)),
  }
}

/** Smoothly moves `value` towards `target` at `rate` per second. */
export function approach(value: number, target: number, rate: number, dt: number): number {
  return target + (value - target) * Math.exp(-rate * dt)
}
