/**
 * When the animation runs, pauses, or shows a single still frame. Pure, so every transition is
 * testable; the loop in `loop.ts` applies it.
 */

export interface GateInput {
  /** `document.hidden`: the tab is in the background. */
  hidden: boolean
  /** `prefers-reduced-motion: reduce`. */
  reducedMotion: boolean
  /** Number of stages intersecting the viewport. */
  visibleStages: number
  /** The WebGL context was lost and not restored. */
  contextLost: boolean
}

export type GateState = 'run' | 'still' | 'idle' | 'off'

/**
 * - `run`: animate every frame;
 * - `still`: render one frame with no motion, then stop (reduced motion);
 * - `idle`: nothing to draw now (hidden tab or no stage on screen), keep listeners;
 * - `off`: nothing can be drawn (context lost); the CSS fallback shows.
 */
export function gateState(input: GateInput): GateState {
  if (input.contextLost) return 'off'
  if (input.hidden || input.visibleStages === 0) return 'idle'
  return input.reducedMotion ? 'still' : 'run'
}

/** Device pixels per CSS pixel to render at: the real ratio, capped at 2. */
export function renderScale(devicePixelRatio: number, cap = 2): number {
  if (!Number.isFinite(devicePixelRatio) || devicePixelRatio <= 0) return 1
  return Math.min(cap, devicePixelRatio)
}

/** Canvas size in device px for a CSS size; never 0, never above `max` per side. */
export function canvasSize(cssW: number, cssH: number, scale: number, max = 4096): { w: number; h: number } {
  return {
    w: Math.max(1, Math.min(max, Math.round(cssW * scale))),
    h: Math.max(1, Math.min(max, Math.round(cssH * scale))),
  }
}

/** Frame delta in seconds, clamped so a pause (tab switch, long task) never explodes the springs. */
export function frameDelta(nowMs: number, lastMs: number, maxS = 0.05): number {
  const dt = (nowMs - lastMs) / 1000
  if (!(dt > 0)) return 0
  return Math.min(maxS, dt)
}
