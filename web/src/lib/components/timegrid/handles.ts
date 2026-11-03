/**
 * The resize handles of a clip or a note (#612): a band the width of the grab zone along an end, in the accent
 * colour, with a bar on the outer edge. The pointer's end is drawn stronger than the other, which stays faint.
 */

export type HandleSide = 'start' | 'end'

export interface HandleSpec {
  /** The block's left and right edge, in CSS pixels. */
  x0: number
  x1: number
  y: number
  height: number
  /** The width of the grab zone (the band). */
  zone: number
  /** The end the pointer is on, or the end being dragged; the other one is drawn faint. `null` draws both faint. */
  strong: HandleSide | null
  /** The ends that resize; both by default (a note resizes at its end only). */
  sides?: readonly HandleSide[]
}

/** Band and bar opacity of a faint and of a strong handle. */
export const HANDLE_ALPHA = { faintBand: 0.16, strongBand: 0.38, faintBar: 0.55, strongBar: 1 } as const

/** Draws the two handles of a block; the block's own drawing is already on the canvas. */
export function drawHandles(ctx: CanvasRenderingContext2D, accent: string, spec: HandleSpec): void {
  const { x0, x1, y, height, strong } = spec
  const zone = Math.max(2, Math.min(spec.zone, (x1 - x0) / 2))
  ctx.save()
  ctx.fillStyle = accent
  for (const side of spec.sides ?? (['start', 'end'] as const)) {
    const on = strong === side
    const bx = side === 'start' ? x0 : x1 - zone
    ctx.globalAlpha = on ? HANDLE_ALPHA.strongBand : HANDLE_ALPHA.faintBand
    ctx.fillRect(bx, y, zone, height)
    const barW = on ? 3 : 2
    ctx.globalAlpha = on ? HANDLE_ALPHA.strongBar : HANDLE_ALPHA.faintBar
    ctx.fillRect(side === 'start' ? x0 : x1 - barW, y, barW, height)
  }
  ctx.restore()
}
