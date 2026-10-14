/**
 * The landing page's living sound objects: WebGL2 shaders (OGL) behind the hero and the modules
 * of the library, loaded lazily by the inline bootstrap (`scripts.ts`) once a `[data-fk-anim]`
 * element nears the viewport. Everything here is progressive enhancement: without it, or if
 * anything fails, the prerendered CSS objects stay as they are.
 *
 * One WebGL context serves every stage; each stage blits its frame onto its own 2D canvas. The
 * loop runs only while a stage is on screen and the tab visible, renders a single still frame
 * under `prefers-reduced-motion: reduce`, and tears everything down if the context is lost.
 *
 * Verification harness: `?fk-anim-t=<seconds>[&fk-anim-p=<x>,<y>]` steps the animation with a
 * fixed 1/60 s timestep to that time, with the pointer at viewport (x, y), renders once and
 * stops; `<html data-fk-anim-ready>` then marks the frame as reproducible.
 */

import { gateState, renderScale, type GateInput } from './gate'
import { Loop } from './loop'
import { Pointer } from './pointer'
import { createStage, isKind, Renderer, type LocalPointer, type Stage } from './stages'

/** Pointer influence reaches this far outside a stage, in CSS px. */
const MARGIN = 80

/** The seek harness's request, from the page's query string. */
export function seekRequest(search: string): { t: number; x: number; y: number } | undefined {
  const q = new URLSearchParams(search)
  const t = Number(q.get('fk-anim-t'))
  if (!q.has('fk-anim-t') || !Number.isFinite(t) || t < 0) return undefined
  const [x, y] = (q.get('fk-anim-p') ?? '-1000,-1000').split(',').map(Number)
  return { t: Math.min(t, 120), x: Number.isFinite(x) ? x : -1000, y: Number.isFinite(y) ? y : -1000 }
}

export function mount(root: ParentNode = document): () => void {
  const els = [...root.querySelectorAll<HTMLElement>('[data-fk-anim]')]
  if (els.length === 0) return () => {}
  let renderer: Renderer
  try {
    renderer = new Renderer()
  } catch {
    return () => {}
  }
  const stages: Stage[] = []
  for (const el of els) {
    const kind = el.dataset.fkAnim
    if (!isKind(kind)) continue
    try {
      stages.push(createStage(renderer, el, kind, stages.length))
    } catch {
      // That stage keeps its CSS object: nothing is put in the page before a stage is built.
    }
  }
  if (stages.length === 0) return () => {}

  const pointer = new Pointer()
  pointer.attach()
  const reduced = matchMedia('(prefers-reduced-motion: reduce)')
  const input: GateInput = { hidden: document.hidden, reducedMotion: reduced.matches, visibleStages: 0, contextLost: false }
  const local: LocalPointer = { x: 0, y: 0, vx: 0, vy: 0, weight: 0, over: false, down: false, idleFor: 0 }

  const frame = (t: number, dt: number, still = false) => {
    if (renderer.lost) {
      input.contextLost = true
      apply()
      return
    }
    const now = performance.now()
    pointer.update(now)
    for (const stage of stages) {
      if (!stage.visible && !still) continue
      const rect = stage.rect()
      local.x = pointer.x - rect.left
      local.y = pointer.y - rect.top
      local.vx = pointer.vx
      local.vy = pointer.vy
      local.down = pointer.down
      local.idleFor = (now - pointer.lastMove) / 1000
      local.over =
        pointer.present &&
        local.x > -MARGIN &&
        local.y > -MARGIN &&
        local.x < rect.width + MARGIN &&
        local.y < rect.height + MARGIN
      local.weight = stage.weight
      try {
        stage.render(t, dt, local, still)
      } catch {
        input.contextLost = true
        apply()
        return
      }
    }
  }
  const loop = new Loop(frame)

  const resizeAll = () => {
    const scale = renderScale(devicePixelRatio)
    for (const stage of stages) stage.resize(scale)
    if (!loop.running) frame(loop.t, 0, true)
  }

  const apply = () => {
    switch (gateState(input)) {
      case 'run':
        loop.start()
        break
      case 'still':
        loop.stop()
        frame(0, 0, true)
        break
      case 'idle':
        loop.stop()
        break
      case 'off':
        destroy()
        break
    }
  }

  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        const stage = stages.find((s) => s.el === e.target)
        if (stage) stage.visible = e.isIntersecting
      }
      input.visibleStages = stages.filter((s) => s.visible).length
      apply()
    },
    { rootMargin: '48px' },
  )
  const ro = new ResizeObserver(() => resizeAll())
  const onVisibility = () => {
    input.hidden = document.hidden
    apply()
  }
  const onReduced = () => {
    input.reducedMotion = reduced.matches
    apply()
  }

  function destroy() {
    loop.stop()
    io.disconnect()
    ro.disconnect()
    pointer.detach()
    document.removeEventListener('visibilitychange', onVisibility)
    reduced.removeEventListener('change', onReduced)
    for (const stage of stages) stage.destroy()
  }

  const seek = seekRequest(location.search)
  if (seek) {
    // Deterministic frame for screenshots: no observers, no loop, a fixed timestep.
    const scale = renderScale(devicePixelRatio)
    for (const stage of stages) {
      stage.resize(scale)
      stage.visible = true
    }
    pointer.detach()
    pointer.x = seek.x
    pointer.y = seek.y
    pointer.present = seek.x > -999
    pointer.lastMove = performance.now()
    const step = 1 / 60
    const n = Math.round(seek.t / step)
    for (let i = 0; i <= n; i++) frame(i * step, i === 0 ? 0 : step)
    document.documentElement.dataset.fkAnimReady = ''
    return destroy
  }

  for (const stage of stages) {
    io.observe(stage.el)
    ro.observe(stage.el)
  }
  document.addEventListener('visibilitychange', onVisibility)
  reduced.addEventListener('change', onReduced)
  resizeAll()
  return destroy
}
