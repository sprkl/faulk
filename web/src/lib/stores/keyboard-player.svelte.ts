import { clampOctave, defaultOctave, isTextTarget, noteForKey, octaveStep, playsNote, TYPING_TOGGLE_CODE } from '../keyboard'
import { isStepEntryToggle } from '../../views/piano-roll/step-entry'
import { instrumentKind, isMidi, trackLabel } from '../project/tracks'
import { inputNoteOff, inputNoteOn } from '../synth-commands'
import type { Dispatcher } from './dispatcher'
import type { EngineStore } from './engine.svelte'
import type { ProjectStore } from './project.svelte'
import type { UiStore } from './ui.svelte'

/**
 * The computer keyboard as a piano, in every view (#361): the shell listens to the window and hands the
 * events here. The key-sharing rules are `playsNote` (`lib/keyboard.ts`); a note goes through the command
 * layer (`live.input_note_on`), which routes it to the selected MIDI track and the armed ones.
 */
export class KeyboardPlayer {
  /** Notes held on the computer keyboard, by physical key, so a change of octave releases the right note. */
  // Not reactive on purpose: nothing renders from it.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  private held = new Map<string, number>()

  constructor(
    private project: ProjectStore,
    private engine: EngineStore,
    private ui: UiStore,
    private dispatcher: Dispatcher,
  ) {}

  /** The selected track, when it is a MIDI track: the live input goes to it and to the armed tracks. */
  target = $derived.by(() => (this.project.selected && isMidi(this.project.selected.track) ? this.project.selected : null))
  /** An armed MIDI track other than the target: the command layer plays only MIDI tracks, whatever a command armed. */
  private armedMidi = $derived.by(() => this.project.armed.filter((id) => this.project.tracks.some((entry) => entry.id === id && isMidi(entry.track))))
  private armedElsewhere = $derived.by(() => this.armedMidi.some((id) => id !== this.target?.id))
  /** What the keyboard plays now, in words. */
  targetHint = $derived.by(() => {
    const t = this.ui.t
    const target = this.target
    if (target) return t(this.armedElsewhere ? 'keys.targetArmed' : 'keys.target', { name: trackLabel(target, t) })
    return t(this.armedElsewhere ? 'keys.armedOnly' : 'keys.noTarget')
  })
  /** The kind of the target's instrument; the octave starts where its sounds are when it changes (drums or not). */
  private targetKind = $derived.by(() => (this.target ? instrumentKind(this.target.track) : null))
  /** Changes with the kind only: switching between tracks of the same kind keeps the octave the user chose. */
  octave = $derived.by(() => defaultOctave(this.targetKind))

  releaseAll() {
    for (const note of this.held.values()) this.dispatcher.run(inputNoteOff(note))
    this.held.clear()
  }

  /** Switches musical typing; the held notes end, so none outlives the mode that played it. */
  toggleTyping() {
    this.releaseAll()
    this.ui.typing = this.ui.typing === 'notes' ? 'shortcuts' : 'notes'
    // Letters left to the shortcuts write nothing either: step entry (#882) ends with typing, not left as a mute mode.
    if (this.ui.typing === 'shortcuts' && this.ui.stepEntry) {
      this.ui.stepEntry = false
      this.ui.announceHistory(this.ui.t('keys.stepStopped'))
    }
  }

  /** Ends step entry from outside the roll (#916): said where the page listens. */
  private disarmStepEntry() {
    this.ui.stepEntry = false
    this.ui.announceHistory(this.ui.t('keys.stepStopped'))
  }

  keyDown(event: KeyboardEvent) {
    // Step entry armed with no clip to write into (the roll has none open or is not shown): Alt+E and Escape end it from here; with a
    // clip the roll takes both (#916).
    if (this.ui.stepEntry && !this.ui.stepWriting && !event.defaultPrevented && !isTextTarget(event.target) && isStepEntryToggle(event as never)) {
      event.preventDefault?.()
      this.disarmStepEntry()
      return
    }
    if (!playsNote(event)) return
    if (event.code === 'Escape' && this.ui.stepEntry) {
      this.disarmStepEntry()
      return
    }
    if (event.code === TYPING_TOGGLE_CODE && !event.shiftKey) {
      this.toggleTyping()
      return
    }
    // Musical typing off: the letters are no notes (the MIDI controller and the on-screen keys are not concerned).
    // Step entry writes notes with no engine started, so the octave keys must work then too (#882).
    const writes = this.ui.stepEntry && this.ui.stepWriting
    if ((!this.engine.started && !writes) || this.ui.typing !== 'notes') return
    const step = octaveStep(event.code)
    if (step !== 0) {
      this.releaseAll()
      this.octave = clampOctave(this.octave + step)
      return
    }
    // Step entry (#882): the piano roll writes the letters as notes; none plays live (Z and X above still move the octave).
    // Armed with no clip open (#916), nothing writes them, so they play.
    if (writes) return
    const note = noteForKey(event.code, this.octave)
    if (note === null || this.held.has(event.code)) return
    // The command layer routes the note: the selected MIDI track and the armed ones (none: nothing plays).
    // It sends the note off where the note on went, so a track change while the key is held ends it.
    if (!this.target && this.armedMidi.length === 0) return
    this.held.set(event.code, note)
    // Stamped with the key's time, so a recording places it where it was played (#499).
    this.dispatcher.runPlayed(inputNoteOn(note, 100), event.timeStamp)
  }

  keyUp(event: KeyboardEvent) {
    const note = this.held.get(event.code)
    if (note === undefined) return
    this.held.delete(event.code)
    this.dispatcher.runPlayed(inputNoteOff(note), event.timeStamp)
  }
}
