/**
 * One stage per animated element: a 2D canvas over the element, painted from the page's single
 * WebGL2 context (OGL), and the state of its object. Each stage draws into the shared GL canvas
 * and blits the result onto its own canvas.
 */

import { Flowmap, Geometry, GPGPU, Mesh, Program, Renderer as OglRenderer, Texture, Triangle } from 'ogl'
import type { OGLRenderingContext } from 'ogl'
import { DropRing, HitClock } from './clocks'
import { createFocus, packFoci, stepFoci, type Focus } from './foci'
import { canvasSize } from './gate'
import { DEFAULT_PARTICLE_PARAMS, particleConstants, type Box, type PointerState } from './particles'
import { approach, idlePoint } from './pointer'
import { VERTEX } from './shaders/common'
import { HERO, PARTICLE_FRAGMENT, PARTICLE_VERTEX, SIM, SIM_VERTEX } from './shaders/hero'
import { CRUSH, ECHO, HAZE, KICK } from './shaders/objects'

export type Kind = 'hero' | 'kick' | 'haze' | 'crush' | 'echo'

export const KINDS: readonly Kind[] = ['hero', 'kick', 'haze', 'crush', 'echo']

export function isKind(value: string | null | undefined): value is Kind {
  return (KINDS as readonly string[]).includes(value ?? '')
}

/** The pointer as a stage sees it: local CSS px, velocity, and how much it counts right now. */
export interface LocalPointer extends PointerState {
  /** Whether a real pointer is over the stage (with a margin), before smoothing. */
  over: boolean
  /** Pointer pressed. */
  down: boolean
  /** Seconds since the pointer last moved anywhere. */
  idleFor: number
}

type Uniforms = Record<string, { value: unknown }>

export class Renderer {
  readonly ogl: OglRenderer
  readonly gl: OGLRenderingContext
  readonly canvas: HTMLCanvasElement
  readonly triangle: Triangle
  /** Float render targets (flow maps, particle simulation) are available. */
  readonly float: boolean
  /** A 1 × 1 empty flow map for stages without one. */
  readonly noFlow: Texture
  lost = false

  constructor() {
    this.ogl = new OglRenderer({
      width: 2,
      height: 2,
      dpr: 1,
      alpha: true,
      premultipliedAlpha: true,
      antialias: false,
      depth: false,
      stencil: false,
      autoClear: false,
      powerPreference: 'low-power',
    })
    if (!this.ogl.isWebgl2) throw new Error('WebGL2 unavailable')
    this.gl = this.ogl.gl
    this.canvas = this.gl.canvas as HTMLCanvasElement
    this.triangle = new Triangle(this.gl)
    this.float = !!this.ogl.getExtension('EXT_color_buffer_float')
    this.noFlow = new Texture(this.gl, { image: new Uint8Array(4), width: 1, height: 1, generateMipmaps: false })
    this.canvas.addEventListener('webglcontextlost', () => {
      this.lost = true
    })
  }

  /** A fragment shader over the whole viewport. */
  quad(fragment: string, uniforms: Uniforms): Mesh {
    const program = new Program(this.gl, { vertex: VERTEX, fragment, uniforms, depthTest: false, depthWrite: false })
    assertLinked(this.gl, program)
    return new Mesh(this.gl, { geometry: this.triangle, program })
  }

  /** Before offscreen passes (flow map, simulation): no scissor from a previous stage. */
  offscreen(): void {
    this.gl.disable(this.gl.SCISSOR_TEST)
  }

  /** Grows the GL canvas to at least `w` × `h` and targets its bottom-left corner. */
  begin(w: number, h: number): void {
    const { canvas, gl, ogl } = this
    if (canvas.width < w || canvas.height < h) ogl.setSize(Math.max(canvas.width, w), Math.max(canvas.height, h))
    ogl.bindFramebuffer()
    ogl.setViewport(w, h, 0, 0)
    ogl.setScissor(w, h, 0, 0)
    ogl.enable(gl.SCISSOR_TEST)
  }

  /** Copies the rendered `w` × `h` region onto a stage's canvas. */
  blit(ctx: CanvasRenderingContext2D, w: number, h: number, clear: boolean): void {
    if (clear) ctx.clearRect(0, 0, w, h)
    ctx.drawImage(this.canvas, 0, this.canvas.height - h, w, h, 0, 0, w, h)
  }
}

/**
 * Throws when a program did not link: OGL only warns, and a stage drawing with it would be an
 * empty rectangle over its hidden CSS object. Thrown while the stage is built, so the stage is
 * dropped and keeps its CSS object.
 */
export function assertLinked(gl: Pick<WebGL2RenderingContext, 'getProgramParameter' | 'LINK_STATUS'>, program: Pick<Program, 'program'>): void {
  if (!gl.getProgramParameter(program.program, gl.LINK_STATUS)) throw new Error('shader program did not link')
}

export abstract class Stage {
  readonly canvas: HTMLCanvasElement
  readonly ctx: CanvasRenderingContext2D
  /** CSS size and device size. */
  cssW = 1
  cssH = 1
  w = 1
  h = 1
  scale = 1
  visible = false
  /** Smoothed pointer weight, 0..1. */
  weight = 0
  /** A phase so stages of the same kind are never in step. */
  readonly phase: number
  /** Uniforms every object shares; see `shaders/common.ts`. */
  protected readonly base: Uniforms

  constructor(
    protected readonly r: Renderer,
    readonly el: HTMLElement,
    readonly kind: Kind,
    readonly transparent: boolean,
    index: number,
  ) {
    this.phase = index * 0.37
    this.base = {
      S: { value: new Float32Array(2) },
      T: { value: 0 },
      M: { value: new Float32Array(2) },
      MI: { value: 0 },
      DP: { value: 1 },
      tFlow: { value: r.noFlow },
      FLOW: { value: 0 },
    }
    this.canvas = document.createElement('canvas')
    this.canvas.className = 'fk-anim-canvas'
    this.canvas.setAttribute('aria-hidden', 'true')
    const ctx = this.canvas.getContext('2d', { alpha: transparent })
    if (!ctx) throw new Error('2d context unavailable')
    this.ctx = ctx
  }

  /** Whether `attach` marks the element as drawing its own particles. */
  protected withParticles = false

  /**
   * Puts the canvas in the page and hides the CSS object (`.is-live`). Called once the stage is
   * fully built (`createStage`), so a stage that fails while being built leaves the page untouched.
   */
  attach(): this {
    this.el.prepend(this.canvas)
    this.el.classList.add('is-live')
    if (this.withParticles) this.el.classList.add('has-particles')
    return this
  }

  /** The canvas's rectangle in the viewport (it may overflow its element, as in the Atelier). */
  rect(): DOMRect {
    return this.canvas.getBoundingClientRect()
  }

  /** Reads the canvas's size and sizes it for `scale` device px per CSS px. */
  resize(scale: number): void {
    this.scale = scale
    const rect = this.rect()
    this.cssW = Math.max(1, rect.width)
    this.cssH = Math.max(1, rect.height)
    const size = canvasSize(this.cssW, this.cssH, scale)
    this.w = size.w
    this.h = size.h
    if (this.canvas.width !== size.w) this.canvas.width = size.w
    if (this.canvas.height !== size.h) this.canvas.height = size.h
    this.measured()
  }

  /** Hook for stages that measure child elements. */
  protected measured(): void {}

  /** Advances the smoothed pointer weight. */
  protected weigh(p: LocalPointer, dt: number): number {
    this.weight = approach(this.weight, p.over ? 1 : 0, p.over ? 10 : 4, dt)
    return this.weight
  }

  protected common(t: number, p: PointerState): void {
    const s = this.base.S.value as Float32Array
    s[0] = this.cssW
    s[1] = this.cssH
    this.base.T.value = t + this.phase
    const m = this.base.M.value as Float32Array
    m[0] = p.x
    m[1] = p.y
    this.base.MI.value = this.weight
    this.base.DP.value = this.scale
  }

  /** Draws `mesh` over the stage and blits it. */
  protected paint(mesh: Mesh): void {
    this.r.begin(this.w, this.h)
    if (this.transparent) {
      this.r.gl.clearColor(0, 0, 0, 0)
      this.r.gl.clear(this.r.gl.COLOR_BUFFER_BIT)
    }
    mesh.draw()
    this.r.blit(this.ctx, this.w, this.h, this.transparent)
  }

  /** Renders one frame; `still` renders the motionless frame (reduced motion). */
  abstract render(t: number, dt: number, p: LocalPointer, still: boolean): void

  destroy(): void {
    this.canvas.remove()
    this.el.classList.remove('is-live', 'has-particles')
  }
}

/** Feeds the pointer into an OGL flow map (uv with y up; velocity in px/ms, as OGL expects). */
function stir(flow: Flowmap, stage: Stage, p: PointerState, dt: number): void {
  flow.aspect = stage.cssW / stage.cssH
  const on = p.weight > 0.01
  flow.mouse.set(on ? p.x / stage.cssW : -10, on ? 1 - p.y / stage.cssH : -10)
  const k = Math.min(1.4, Math.hypot(p.vx, p.vy) / 900) * p.weight
  const n = Math.hypot(p.vx, p.vy) || 1
  flow.velocity.set((p.vx / n) * k, (-p.vy / n) * k)
  // Dissipation per frame, made independent of the frame rate.
  flow.mesh.program.uniforms.uDissipation.value = Math.pow(0.984, dt * 60)
  flow.update()
}

const PARTICLES = 16384

export class HeroStage extends Stage {
  private readonly foci: Focus[] = [createFocus(0.28, 0.6, 1.0, 0.55, 0.4), createFocus(0.55, 0.46, 1.7, 0.45, 0.42), createFocus(0.72, 0.3, 2.6, 0.6, 0.3)]
  private readonly glow: Mesh
  private readonly uniforms: Uniforms
  private readonly flow: Flowmap | undefined
  private readonly sim: GPGPU | undefined
  private readonly simUniforms: Uniforms | undefined
  private readonly points: Mesh | undefined
  private panel: Box = { x: 0, y: 0, w: 1, h: 1 }
  private wave: Box = { x: 0, y: 0, w: 1, h: 1 }
  private panelRadius = 14
  private reset = true
  private readonly virtual: PointerState = { x: 0, y: 0, vx: 0, vy: 0, weight: 0 }
  private idleWeight = 0

  constructor(r: Renderer, el: HTMLElement, index: number) {
    super(r, el, 'hero', false, index)
    const { gl } = r
    this.uniforms = {
      ...this.base,
      F: { value: new Array<number>(12).fill(0) },
      P: { value: new Float32Array(4) },
      PR: { value: 14 },
    }
    if (r.float) {
      this.flow = new Flowmap(gl, { size: 128, falloff: 0.28, alpha: 1, dissipation: 0.984 })
      assertLinked(gl, this.flow.mesh.program)
      this.uniforms.tFlow = this.flow.uniform
      this.uniforms.FLOW = { value: 1 }
      // The particle simulation: a GPGPU texture per state, the constants in a second texture.
      this.sim = new GPGPU(gl, { data: new Float32Array(PARTICLES * 4) })
      const size = this.sim.size
      const init = new Texture(gl, {
        image: particleConstants(size, PARTICLES),
        width: size,
        height: size,
        type: gl.FLOAT,
        format: gl.RGBA,
        internalFormat: (gl as unknown as WebGL2RenderingContext).RGBA32F,
        minFilter: gl.NEAREST,
        magFilter: gl.NEAREST,
        generateMipmaps: false,
        flipY: false,
      })
      const k = DEFAULT_PARTICLE_PARAMS
      this.simUniforms = {
        tInit: { value: init },
        B: { value: new Float32Array(4) },
        T: { value: 0 },
        DT: { value: 0 },
        PTR: { value: new Float32Array(4) },
        PW: { value: 0 },
        K: { value: new Float32Array([k.radius, k.push, k.swirl, k.stiffness]) },
        K2: { value: new Float32Array([k.damping, k.wake, k.turbulence]) },
        RESET: { value: 1 },
      }
      assertLinked(gl, this.sim.addPass({ vertex: SIM_VERTEX, fragment: SIM, uniforms: this.simUniforms }).program)
      const program = new Program(gl, {
        vertex: PARTICLE_VERTEX,
        fragment: PARTICLE_FRAGMENT,
        uniforms: { tMap: this.sim.uniform, tInit: { value: init }, S: this.base.S, DP: this.base.DP, TS: { value: size } },
        transparent: true,
        depthTest: false,
        depthWrite: false,
      })
      assertLinked(gl, program)
      program.setBlendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA)
      const geometry = new Geometry(gl, { coords: { size: 2, data: this.sim.coords } })
      this.points = new Mesh(gl, { geometry, program, mode: gl.POINTS })
      this.withParticles = true
    }
    this.glow = r.quad(HERO, this.uniforms)
  }

  protected override measured(): void {
    const base = this.rect()
    const screen = this.el.querySelector('.screen')
    const svg = this.el.querySelector('svg')
    const rel = (r: DOMRect): Box => ({ x: r.left - base.left, y: r.top - base.top, w: r.width, h: r.height })
    this.panel = screen ? rel(screen.getBoundingClientRect()) : { x: 0, y: 0, w: this.cssW, h: this.cssH }
    this.wave = svg ? rel(svg.getBoundingClientRect()) : this.panel
    this.panelRadius = screen ? parseFloat(getComputedStyle(screen).borderRadius) || 14 : 14
    // The waveform re-forms on the new layout.
    this.reset = true
  }

  render(t: number, dt: number, p: LocalPointer, still: boolean): void {
    const tt = still ? 0 : t
    // A virtual pointer takes over when the real one has been away for a while.
    const idle = !still && !p.over && p.idleFor > 2.5
    this.idleWeight = approach(this.idleWeight, idle ? 0.6 : 0, 1.2, dt)
    const real = this.weigh(p, dt)
    let drive: PointerState
    if (this.idleWeight > real) {
      const ip = idlePoint(tt + this.phase, this.panel)
      const prev = { x: this.virtual.x, y: this.virtual.y }
      this.virtual.x = ip.x
      this.virtual.y = ip.y
      this.virtual.vx = dt > 0 ? (ip.x - prev.x) / dt : 0
      this.virtual.vy = dt > 0 ? (ip.y - prev.y) / dt : 0
      this.virtual.weight = this.idleWeight
      drive = this.virtual
    } else {
      drive = { x: p.x, y: p.y, vx: p.vx, vy: p.vy, weight: real }
    }
    this.weight = drive.weight
    if (!still) stepFoci(this.foci, tt, dt, drive.x / this.cssW, drive.y / this.cssH, drive.weight)

    this.r.offscreen()
    if (this.flow && !still) stir(this.flow, this, drive, dt)
    if (this.sim && this.simUniforms) {
      const u = this.simUniforms
      const b = u.B.value as Float32Array
      b[0] = this.wave.x
      b[1] = this.wave.y
      b[2] = this.wave.w
      b[3] = this.wave.h
      u.T.value = tt
      u.DT.value = still ? 0 : dt
      const ptr = u.PTR.value as Float32Array
      ptr[0] = drive.x
      ptr[1] = drive.y
      ptr[2] = drive.vx
      ptr[3] = drive.vy
      u.PW.value = still ? 0 : drive.weight
      u.RESET.value = this.reset || still ? 1 : 0
      this.reset = false
      this.sim.render()
    }

    this.common(tt, p)
    packFoci(this.foci, this.uniforms.F.value as number[])
    const panel = this.uniforms.P.value as Float32Array
    panel[0] = this.panel.x
    panel[1] = this.panel.y
    panel[2] = this.panel.w
    panel[3] = this.panel.h
    this.uniforms.PR.value = this.panelRadius
    this.r.begin(this.w, this.h)
    this.glow.draw()
    this.points?.draw()
    this.r.blit(this.ctx, this.w, this.h, false)
  }
}

/** The object's centre and radius, from the element the canvas replaces. */
function objectGeometry(el: HTMLElement, stage: Stage): { cx: number; cy: number; r: number } {
  const object = el.querySelector<HTMLElement>('.object')
  if (!object) return { cx: stage.cssW / 2, cy: stage.cssH / 2, r: Math.min(stage.cssW, stage.cssH) * 0.29 }
  const base = stage.rect()
  const r = object.getBoundingClientRect()
  return { cx: r.left - base.left + r.width / 2, cy: r.top - base.top + r.height / 2, r: Math.min(r.width, r.height) / 2 }
}

export class KickStage extends Stage {
  private readonly clock: HitClock
  private readonly mesh: Mesh
  private readonly uniforms: Uniforms
  private cx = 0
  private cy = 0
  private r0 = 60
  private pvx = 0
  private pvy = 0
  private pvvx = 0
  private pvvy = 0
  private wasInside = false

  constructor(r: Renderer, el: HTMLElement, index: number, transparent: boolean) {
    super(r, el, 'kick', transparent, index)
    this.clock = new HitClock(1.2, 0.3 + index * 0.4)
    this.uniforms = {
      ...this.base,
      C: { value: new Float32Array(2) },
      R: { value: 60 },
      H: { value: 1 },
      PV: { value: new Float32Array(2) },
      BG: { value: transparent ? 0 : 1 },
    }
    this.mesh = r.quad(KICK, this.uniforms)
  }

  protected override measured(): void {
    const g = this.transparent ? { cx: this.cssW / 2, cy: this.cssH / 2, r: Math.min(this.cssW, this.cssH) / 2 - 28 } : objectGeometry(this.el, this)
    this.cx = g.cx
    this.cy = g.cy
    this.r0 = g.r
  }

  render(t: number, dt: number, p: LocalPointer, still: boolean): void {
    let since = 1
    if (!still) {
      this.clock.tick(t)
      this.weigh(p, dt)
      // The pointer pushes the body, with a spring's lag; entering it (or pressing) hits it.
      const dx = p.x - this.cx
      const dy = p.y - this.cy
      const d = Math.hypot(dx, dy)
      const inside = p.over && d < this.r0
      if ((inside && !this.wasInside) || (inside && p.down)) this.clock.hit(t)
      this.wasInside = inside
      const reach = this.r0 * 2.4
      const prox = p.over && d < reach ? 1 - d / reach : 0
      const tx = d > 1 ? (-dx / d) * prox * 14 : 0
      const ty = d > 1 ? (-dy / d) * prox * 14 : 0
      this.pvvx += ((tx - this.pvx) * 60 - this.pvvx * 8) * dt
      this.pvvy += ((ty - this.pvy) * 60 - this.pvvy * 8) * dt
      this.pvx += this.pvvx * dt
      this.pvy += this.pvvy * dt
      since = this.clock.since(t)
    }
    this.common(still ? 0 : t, p)
    const c = this.uniforms.C.value as Float32Array
    c[0] = this.cx
    c[1] = this.cy
    this.uniforms.R.value = this.r0
    this.uniforms.H.value = since
    const pv = this.uniforms.PV.value as Float32Array
    pv[0] = still ? 0 : this.pvx
    pv[1] = still ? 0 : this.pvy
    this.paint(this.mesh)
  }
}

export class HazeStage extends Stage {
  private readonly foci: Focus[] = [createFocus(0.4, 0.55, 1.4, 0.5, 0.4), createFocus(0.62, 0.42, 2.2, 0.4, 0.36), createFocus(0.55, 0.6, 3.0, 0.7, 0.2)]
  private readonly mesh: Mesh
  private readonly uniforms: Uniforms
  private readonly flow: Flowmap | undefined

  constructor(r: Renderer, el: HTMLElement, index: number) {
    super(r, el, 'haze', false, index)
    this.uniforms = { ...this.base, F: { value: new Array<number>(12).fill(0) } }
    if (r.float) {
      this.flow = new Flowmap(r.gl, { size: 64, falloff: 0.32, alpha: 0.8, dissipation: 0.99 })
      assertLinked(r.gl, this.flow.mesh.program)
      this.uniforms.tFlow = this.flow.uniform
      this.uniforms.FLOW = { value: 1 }
    }
    this.mesh = r.quad(HAZE, this.uniforms)
  }

  render(t: number, dt: number, p: LocalPointer, still: boolean): void {
    const tt = still ? 0 : t
    if (!still) {
      const w = this.weigh(p, dt)
      stepFoci(this.foci, tt, dt, p.x / this.cssW, p.y / this.cssH, w, {
        stiffness: 2.2,
        damping: 1.1,
        orbit: 0.12,
        wander: 0.07,
      })
      if (this.flow) {
        this.r.offscreen()
        stir(this.flow, this, { x: p.x, y: p.y, vx: p.vx, vy: p.vy, weight: w }, dt)
      }
    }
    this.common(tt, p)
    packFoci(this.foci, this.uniforms.F.value as number[])
    this.paint(this.mesh)
  }
}

export class CrushStage extends Stage {
  private readonly mesh: Mesh
  private readonly uniforms: Uniforms
  private cx = 0
  private cy = 0
  private r0 = 70
  private crush = 0.2

  constructor(r: Renderer, el: HTMLElement, index: number) {
    super(r, el, 'crush', false, index)
    this.uniforms = {
      ...this.base,
      C: { value: new Float32Array(2) },
      R: { value: 70 },
      CR: { value: 0.18 },
      TS: { value: 0 },
    }
    this.mesh = r.quad(CRUSH, this.uniforms)
  }

  protected override measured(): void {
    const g = objectGeometry(this.el, this)
    this.cx = g.cx
    this.cy = g.cy
    this.r0 = g.r
  }

  render(t: number, dt: number, p: LocalPointer, still: boolean): void {
    if (!still) {
      const w = this.weigh(p, dt)
      const d = Math.hypot(p.x - this.cx, p.y - this.cy)
      const reach = this.r0 * 2.6
      const prox = d < reach ? 1 - d / reach : 0
      // More crushing near the pointer, with inertia.
      this.crush = approach(this.crush, 0.18 + 0.82 * prox * w, 3, dt)
    }
    this.common(still ? 0 : t, p)
    const c = this.uniforms.C.value as Float32Array
    c[0] = this.cx
    c[1] = this.cy
    this.uniforms.R.value = this.r0
    this.uniforms.CR.value = still ? 0.18 : this.crush
    // Stepped time: 2.5 changes per second, under the WCAG limit of 3.
    this.uniforms.TS.value = still ? 0 : Math.floor((t + this.phase) * 2.5) / 2.5
    this.paint(this.mesh)
  }
}

export class EchoStage extends Stage {
  private readonly drops = new DropRing(12, 0.9)
  private readonly mesh: Mesh
  private readonly uniforms: Uniforms
  private cx = 0
  private cy = 0
  private r0 = 85

  constructor(r: Renderer, el: HTMLElement, index: number) {
    super(r, el, 'echo', false, index)
    this.uniforms = {
      ...this.base,
      C: { value: new Float32Array(2) },
      R: { value: 85 },
      D: { value: this.drops.data },
    }
    this.mesh = r.quad(ECHO, this.uniforms)
  }

  protected override measured(): void {
    const g = objectGeometry(this.el, this)
    this.cx = g.cx
    this.cy = g.cy
    this.r0 = g.r
  }

  render(t: number, dt: number, p: LocalPointer, still: boolean): void {
    if (!still) {
      const w = this.weigh(p, dt)
      this.drops.tick(t)
      if (w > 0.3) {
        const side = Math.min(this.cssW, this.cssH)
        this.drops.pointer(t, p.x / this.cssW, p.y / this.cssH, (Math.hypot(p.vx, p.vy) / side) * w)
      }
    }
    this.common(still ? 0 : t, p)
    const c = this.uniforms.C.value as Float32Array
    c[0] = this.cx
    c[1] = this.cy
    this.uniforms.R.value = this.r0
    this.paint(this.mesh)
  }
}

export function createStage(r: Renderer, el: HTMLElement, kind: Kind, index: number): Stage {
  return buildStage(r, el, kind, index).attach()
}

function buildStage(r: Renderer, el: HTMLElement, kind: Kind, index: number): Stage {
  switch (kind) {
    case 'hero':
      return new HeroStage(r, el, index)
    case 'kick':
      return new KickStage(r, el, index, el.dataset.fkAnimBg === 'transparent')
    case 'haze':
      return new HazeStage(r, el, index)
    case 'crush':
      return new CrushStage(r, el, index)
    case 'echo':
      return new EchoStage(r, el, index)
  }
}
