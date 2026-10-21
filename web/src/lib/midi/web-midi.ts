import type { Command, MidiInput } from '../generated/protocol'
import { decodeMidi } from './decode'
import { allNotesOff } from '../synth-commands'

export interface WebMidiEvents {
  onInputs(inputs: MidiInput[], selected: string | null): void
  onCommand(command: Command): void
  onActivity(note: number, velocity: number): void
}

/** The parts of the Web MIDI API this host uses (a fake one implements it in tests). */
export interface MidiAccessLike {
  inputs: { forEach(cb: (input: MidiInputLike) => void): void; get(id: string): MidiInputLike | undefined }
  onstatechange: ((event: unknown) => void) | null
}

export interface MidiInputLike {
  id: string
  name: string | null
  state: 'connected' | 'disconnected'
  onmidimessage: ((event: { data: Uint8Array | null }) => void) | null
  close(): Promise<unknown>
}

/** MIDI input in Web mode: lists inputs, follows hot-plug, plays the selected one. */
export class WebMidiHost {
  private access: MidiAccessLike | null = null
  private selected: MidiInputLike | null = null

  constructor(private readonly events: WebMidiEvents) {}

  static supported(): boolean {
    return typeof navigator !== 'undefined' && 'requestMIDIAccess' in navigator
  }

  /** Asks the browser for MIDI access. Returns false when refused or unsupported. */
  async enable(request: () => Promise<MidiAccessLike> = () => navigator.requestMIDIAccess() as Promise<never>) {
    try {
      this.access = await request()
    } catch {
      return false
    }
    this.access.onstatechange = () => this.refresh()
    this.refresh()
    return true
  }

  inputs(): MidiInput[] {
    const list: MidiInput[] = []
    this.access?.inputs.forEach((input) => {
      if (input.state === 'connected') list.push({ id: input.id, name: input.name ?? input.id })
    })
    return list
  }

  /** Called on hot-plug: an unplugged selected input is dropped and its notes released. */
  refresh() {
    if (this.selected && this.selected.state !== 'connected') {
      this.detach()
      this.events.onCommand(allNotesOff())
    }
    this.events.onInputs(this.inputs(), this.selected?.id ?? null)
  }

  select(id: string | null) {
    this.detach()
    const input = id ? this.access?.inputs.get(id) : undefined
    if (input && input.state === 'connected') {
      input.onmidimessage = (event) => {
        if (!event.data) return
        const command = decodeMidi(event.data)
        if (!command) return
        this.events.onCommand(command)
        if (command.type === 'live.input_note_on') this.events.onActivity(command.note, command.velocity)
        if (command.type === 'live.input_note_off') this.events.onActivity(command.note, 0)
      }
      this.selected = input
    }
    this.events.onInputs(this.inputs(), this.selected?.id ?? null)
  }

  /**
   * Closes every input, so the helper can open them: on Windows a MIDI input can be
   * open in one application only.
   */
  async releaseAll() {
    this.detach()
    const closing: Promise<unknown>[] = []
    this.access?.inputs.forEach((input) => closing.push(input.close().catch(() => undefined)))
    await Promise.all(closing)
  }

  private detach() {
    if (this.selected) {
      this.selected.onmidimessage = null
      this.selected = null
      this.events.onCommand(allNotesOff())
    }
  }
}
