<script lang="ts">
  /**
   * The pan of a track or of the master (`track` null), from -1 (left) to 1 (right), centred by default, shared by the mixer and the
   * track list. A `Slider`: magnet on the centre, fine mode, reset to the centre as one command. One
   * drag is one history group (`gesture.ts`). It holds no project state.
   */
  import { onDestroy } from 'svelte'
  import { createGesture } from '../gesture'
  import { formatPan } from '../project/tracks'
  import { useStores } from '../stores'
  import { setMasterPan, setPan } from '../track-commands'
  import { parsePan } from '../value-entry'
  import Slider from './Slider.svelte'

  let { track, pan, label, automated, testid }: { track: string | null; pan: number; label: string; automated?: string; testid?: string } = $props()

  const { dispatcher } = useStores()
  /** The command that sets the pan: a track's, or the master's when `track` is null. */
  const panCommand = (pan: number) => (track === null ? setMasterPan(pan) : setPan(track, pan))
  const gesture = createGesture((command) => dispatcher.run(command))
  onDestroy(() => gesture.end())

  function onposition(position: number) {
    // Three decimals: the fine mode goes below the percent the readout shows.
    const next = position === 0.5 ? 0 : Math.round((position * 2 - 1) * 1000) / 1000
    if (next !== pan) dispatcher.run(panCommand(next))
  }

  let slider: ReturnType<typeof Slider> | undefined = $state()

  /** Opens the field to type the pan (the readout next to the control calls it). */
  export function edit() {
    slider?.edit()
  }

  // Typing a pan: one command, or none when the text is unreadable or the pan is the one it has.
  const entry = {
    text: () => formatPan(pan),
    commit(text: string): boolean {
      const next = parsePan(text)
      if (next === undefined) return false
      if (next !== pan) dispatcher.run(panCommand(next))
      return true
    },
  }

  function onreset() {
    // Already centred: no command, no empty history entry.
    if (pan !== 0) dispatcher.run(panCommand(0))
  }
</script>

<Slider
  bind:this={slider}
  kind="pan"
  {entry}
  {label}
  {automated}
  {testid}
  position={(pan + 1) / 2}
  home={0.5}
  valueText={formatPan(pan)}
  defaultText={formatPan(0)}
  {onposition}
  {onreset}
  onbegin={gesture.begin}
  onend={gesture.end}
/>
