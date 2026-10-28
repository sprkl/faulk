/**
 * The hero's waveform as a cloud of particles. The simulation itself runs on the GPU
 * (`shaders/hero.ts`, `SIM`): one texel per particle, ping-pong render targets. This module holds
 * the pure parts: the particles' constants (seeded, so a frame is reproducible), the waveform,
 * the parameters, and a reference step of one particle that states what the shader computes.
 */

export interface ParticleParams {
  /** Radius of the pointer's influence in px. */
  radius: number
  /** Push away from the pointer, px/s². */
  push: number
  /** Tangential swirl around the pointer, px/s². */
  swirl: number
  /** Spring stiffness towards home, 1/s². */
  stiffness: number
  /** Velocity damping, 1/s. */
  damping: number
  /** How much of the pointer's own velocity the particles inherit, 1/s. */
  wake: number
  /** Curl-noise turbulence near the pointer, px/s². */
  turbulence: number
}

export const DEFAULT_PARTICLE_PARAMS: ParticleParams = {
  radius: 110,
  push: 2400,
  swirl: 1300,
  stiffness: 48,
  damping: 4.6,
  wake: 1.6,
  turbulence: 2200,
}

/** Deterministic pseudo-random in [0, 1): a 32-bit LCG, enough for seeds and spread. */
export function lcg(seed: number): () => number {
  let s = seed >>> 0
  return () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0
    return s / 4294967296
  }
}

/**
 * Per-particle constants as an RGBA float texture of `size` × `size` texels: position along
 * the waveform (0..1), thickness offset (px), tone (0..1), seed (0..1). Texels past `count` stay
 * zero: they sit at the waveform's start, invisible under their neighbours.
 */
export function particleConstants(size: number, count: number, seed = 7): Float32Array {
  const rnd = lcg(seed)
  const data = new Float32Array(size * size * 4)
  for (let i = 0; i < count; i++) {
    // A gaussian-ish thickness (sum of three uniforms) for 82 %, a loose halo for the rest.
    const g = rnd() + rnd() + rnd() - 1.5
    const halo = rnd() < 0.18
    data[i * 4] = rnd()
    data[i * 4 + 1] = halo ? g * 11 : g * 2.2
    data[i * 4 + 2] = halo ? 0.18 + rnd() * 0.3 : 0.5 + rnd() * 0.5
    data[i * 4 + 3] = rnd()
  }
  return data
}

/**
 * The decorative waveform of the hero (the same partials as the prerendered SVG), as a
 * normalised amplitude -1..1 at position `x` (0..1); its partials drift slowly with time so the
 * sound seems to keep playing. `SIM` in `shaders/hero.ts` is its GLSL twin.
 */
export function waveform(x: number, t: number): number {
  const env = Math.exp(-2.2 * x) * (1 - Math.exp(-40 * x))
  const breath = 0.86 + 0.14 * Math.sin(t * 0.9)
  return env * breath * (Math.sin(x * 80 + t * 1.1) * 0.7 + Math.sin(x * 211 - t * 2.3) * 0.25 + Math.sin(x * 23 + t * 0.6) * 0.3)
}

export interface Box {
  x: number
  y: number
  w: number
  h: number
}

/** Home of a particle at `along` (0..1) with thickness `spread`, at time `t`, inside `box`. */
export function homeOf(along: number, spread: number, t: number, box: Box): { x: number; y: number } {
  return { x: box.x + along * box.w, y: box.y + box.h * 0.5 - waveform(along, t) * box.h * 0.39 + spread }
}

export interface Particle {
  x: number
  y: number
  vx: number
  vy: number
}

export interface PointerState {
  x: number
  y: number
  vx: number
  vy: number
  /** 0 when the pointer is absent, 1 when it drives the cloud. */
  weight: number
}

/**
 * One step of one particle, semi-implicit Euler: the reference of what the GPU does for all of
 * them (without the curl-noise term, which has no CPU twin). `dt` in seconds, clamped by the
 * caller.
 */
export function stepParticle(
  p: Particle,
  home: { x: number; y: number },
  pointer: PointerState,
  dt: number,
  params: ParticleParams = DEFAULT_PARTICLE_PARAMS,
): void {
  let ax = (home.x - p.x) * params.stiffness
  let ay = (home.y - p.y) * params.stiffness
  const dx = p.x - pointer.x
  const dy = p.y - pointer.y
  const r = Math.hypot(dx, dy) + 1e-3
  const f = Math.max(0, 1 - r / params.radius)
  const k = f * f * pointer.weight
  if (k > 0) {
    const ux = dx / r
    const uy = dy / r
    ax += ux * k * params.push - uy * k * params.swirl + pointer.vx * k * params.wake
    ay += uy * k * params.push + ux * k * params.swirl + pointer.vy * k * params.wake
  }
  const damp = Math.exp(-params.damping * dt)
  p.vx = (p.vx + ax * dt) * damp
  p.vy = (p.vy + ay * dt) * damp
  p.x += p.vx * dt
  p.y += p.vy * dt
}
