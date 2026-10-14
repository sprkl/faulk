import type { WasmOp } from './wasm-ops'

/** Exports of crates/engine-wasm (plain C ABI, see its lib.rs). */
export interface EngineExports {
  memory: WebAssembly.Memory
  fk_new(sampleRate: number): number
  fk_render(instance: number, frames: number): number
  fk_left_ptr(instance: number): number
  fk_right_ptr(instance: number): number
  fk_note_on(instance: number, note: number, velocity: number): void
  fk_note_off(instance: number, note: number): void
  fk_all_notes_off(instance: number): void
  fk_set_param(instance: number, id: number, value: number): void
  fk_active_voices(instance: number): number
}

/** Largest render quantum the wasm engine accepts (`MAX_FRAMES` in engine-wasm). */
export const MAX_FRAMES = 2048

/**
 * The wasm engine as the worklet drives it. `render` allocates nothing once the views
 * exist: the views are rebuilt only if the wasm memory grows (it does not after init).
 */
export class WasmCore {
  private readonly instance: number
  private left!: Float32Array
  private right!: Float32Array
  /** Peak since the last `takePeak`. */
  private peak = 0

  constructor(
    private readonly exports: EngineExports,
    sampleRate: number,
  ) {
    this.instance = exports.fk_new(sampleRate)
    this.makeViews()
  }

  private makeViews() {
    const buffer = this.exports.memory.buffer
    this.left = new Float32Array(buffer, this.exports.fk_left_ptr(this.instance), MAX_FRAMES)
    this.right = new Float32Array(buffer, this.exports.fk_right_ptr(this.instance), MAX_FRAMES)
  }

  apply(op: WasmOp) {
    const [name, a, b] = op
    switch (name) {
      case 'note_on':
        this.exports.fk_note_on(this.instance, a, b)
        break
      case 'note_off':
        this.exports.fk_note_off(this.instance, a)
        break
      case 'all_off':
        this.exports.fk_all_notes_off(this.instance)
        break
      case 'set_param':
        this.exports.fk_set_param(this.instance, a, b)
        break
    }
  }

  /** Renders into the two output channels; `outRight` may be undefined for mono. */
  render(outLeft: Float32Array, outRight: Float32Array | undefined) {
    const frames = this.exports.fk_render(this.instance, outLeft.length)
    if (this.left.buffer !== this.exports.memory.buffer) this.makeViews()
    let peak = this.peak
    for (let i = 0; i < frames; i++) {
      const l = this.left[i]
      outLeft[i] = l
      if (outRight) outRight[i] = this.right[i]
      const a = l < 0 ? -l : l
      if (a > peak) peak = a
    }
    this.peak = peak
  }

  takePeak(): number {
    const peak = this.peak
    this.peak = 0
    return peak
  }

  activeVoices(): number {
    return this.exports.fk_active_voices(this.instance)
  }
}
