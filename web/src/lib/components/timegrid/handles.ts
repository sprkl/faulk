/**
 * The resize handles of a clip or a note (#612): a striped band the width of the grab zone along an end, in the accent
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

/** Opacity of the band, of its stripes and of the edge bar, faint and strong. */
export const HANDLE_ALPHA = { faintBand: 0.16, strongBand: 0.3, faintStripe: 0.3, strongStripe: 0.7, faintBar: 0.55, strongBar: 1 } as const

/** The distance between two stripes, along the band. */
const STRIPE_GAP = 4

/** Draws the handles of a block; the block's own drawing is already on the canvas. */
export function drawHandles(ctx: CanvasRenderingContext2D, accent: string, spec: HandleSpec): void {
  const { x0, x1, y, height, strong } = spec
  const zone = Math.max(2, Math.min(spec.zone, (x1 - x0) / 2))
  ctx.save()
  ctx.fillStyle = accent
  ctx.strokeStyle = accent
  ctx.lineWidth = 1
  for (const side of spec.sides ?? (['start', 'end'] as const)) {
    const on = strong === side
    const bx = side === 'start' ? x0 : x1 - zone
    ctx.globalAlpha = on ? HANDLE_ALPHA.strongBand : HANDLE_ALPHA.faintBand
    ctx.fillRect(bx, y, zone, height)
    // Diagonal stripes across the band: a grip that reads as one on any clip colour.
    ctx.save()
    ctx.beginPath()
    ctx.rect(bx, y, zone, height)
    ctx.clip()
    ctx.globalAlpha = on ? HANDLE_ALPHA.strongStripe : HANDLE_ALPHA.faintStripe
    ctx.beginPath()
    for (let sx = bx - height; sx < bx + zone; sx += STRIPE_GAP) {
      ctx.moveTo(sx, y + height)
      ctx.lineTo(sx + height, y)
    }
    ctx.stroke()
    ctx.restore()
    const barW = on ? 3 : 2
    ctx.globalAlpha = on ? HANDLE_ALPHA.strongBar : HANDLE_ALPHA.faintBar
    ctx.fillRect(side === 'start' ? x0 : x1 - barW, y, barW, height)
  }
  ctx.restore()
}
