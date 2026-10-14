/**
 * The colour foci of a glow (hero, haze): points with a rest position that lean towards the
 * pointer, each with its own mass, so they lag and overshoot like something fluid rather than
 * following the cursor. Positions are fractions of the stage (0..1, y down).
 */

export interface Focus {
  /** Where it rests, fractions of the stage. */
  rx: number
  ry: number
  /** Current position and velocity (fractions, fractions/s). */
  x: number
  y: number
  vx: number
  vy: number
  /** Heavier foci lag more. */
  mass: number
  /** Share of the way to the pointer it leans, 0..1. */
  lean: number
  /** Radius of its gradient, as a fraction of the stage's shorter side. */
  radius: number
}

export interface FociParams {
  /** Spring towards the target, 1/s². */
  stiffness: number
  /** Damping, 1/s: low keeps inertia. */
  damping: number
  /** Orbital push around the pointer, fractions/s². */
  orbit: number
  /** Amplitude of the autonomous wander, fractions. */
  wander: number
}

export const DEFAULT_FOCI_PARAMS: FociParams = { stiffness: 4.5, damping: 1.6, orbit: 0.22, wander: 0.05 }

export function createFocus(rx: number, ry: number, mass: number, lean: number, radius: number): Focus {
  return { rx, ry, x: rx, y: ry, vx: 0, vy: 0, mass, lean, radius }
}

/**
 * One step. `px, py` is the pointer in stage fractions, `weight` its influence (0 when absent).
 * Without a pointer, every focus wanders slowly around its rest position.
 */
export function stepFoci(
  foci: Focus[],
  t: number,
  dt: number,
  px: number,
  py: number,
  weight: number,
  params: FociParams = DEFAULT_FOCI_PARAMS,
): void {
  for (let i = 0; i < foci.length; i++) {
    const f = foci[i]
    const wx = Math.sin(t * 0.21 + i * 2.1) * params.wander
    const wy = Math.cos(t * 0.17 + i * 1.3) * params.wander
    const tx = f.rx + wx + (px - f.rx) * f.lean * weight
    const ty = f.ry + wy + (py - f.ry) * f.lean * weight
    let ax = ((tx - f.x) * params.stiffness - f.vx * params.damping) / f.mass
    let ay = ((ty - f.y) * params.stiffness - f.vy * params.damping) / f.mass
    if (weight > 0) {
      // Orbit around the pointer: a push perpendicular to the line pointer → focus.
      const dx = f.x - px
      const dy = f.y - py
      const d = Math.hypot(dx, dy) + 0.05
      const o = (params.orbit * weight) / (f.mass * d)
      ax += -dy * o
      ay += dx * o
    }
    f.vx += ax * dt
    f.vy += ay * dt
    f.x += f.vx * dt
    f.y += f.vy * dt
  }
}

/** Packs foci as x, y, radius, 0 for a `vec4[]` uniform (a plain array: OGL's array uniforms). */
export function packFoci(foci: Focus[], out: number[]): number[] {
  for (let i = 0; i < foci.length; i++) {
    out[i * 4] = foci[i].x
    out[i * 4 + 1] = foci[i].y
    out[i * 4 + 2] = foci[i].radius
    out[i * 4 + 3] = 0
  }
  return out
}
