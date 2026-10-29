<script lang="ts">
  /**
   * The volume control of a track or of the master, in dB, on the taper of `fader.ts` (0 dB at
   * 78 % of the travel), shared by the mixer's fader and the track list. A `Slider`: magnet on 0 dB,
   * fine mode, reset to 0 dB as one command. One drag is one history group (`gesture.ts`). It sends
   * the command `set` builds and holds no project state.
   */
  import { onDestroy } from 'svelte'
  import { createGesture } from '../gesture'
  import type { Command } from '../generated/protocol'
  import { gainAt, positionOf, UNITY_POSITION } from '../project/fader'
  import { formatGain } from '../project/tracks'
  import { useStores } from '../stores'
  import { parseGain } from '../value-entry'
  import Slider from './Slider.svelte'

  let {
    gainDb,
    label,
    set,
    orientation = 'horizontal',
    automated,
    testid,
  }: {
    gainDb: number | null | undefined
    label: string
    set: (gainDb: number | null) => Command
    orientation?: 'horizontal' | 'vertical'
    /** The tooltip when an automation lane drives the volume (#388). */
    automated?: string
    testid?: string
  } = $props()

  const { dispatcher } = useStores()
  const gesture = createGesture((command) => dispatcher.run(command))
  onDestroy(() => gesture.end())

  const same = (a: number | null | undefined, b: number | null) => (a ?? null) === b

  function onposition(position: number) {
    const gain = gainAt(position)
    if (!same(gainDb, gain)) dispatcher.run(set(gain))
  }

  let slider: ReturnType<typeof Slider> | undefined = $state()

  /** Opens the field to type the gain (the readout next to the fader calls it). */
  export function edit() {
    slider?.edit()
  }

  // Typing a gain: one command, or none when the text is unreadable or the gain is the one it has.
  const entry = {
    text: () => `${formatGain(gainDb)} dB`,
    commit(text: string): boolean {
      const gain = parseGain(text)
      if (gain === undefined) return false
      if (!same(gainDb, gain)) dispatcher.run(set(gain))
      return true
    },
  }

  function onreset() {
    // Already at 0 dB: no command, no empty history entry.
    if (gainDb !== 0) dispatcher.run(set(0))
  }
</script>

<Slider
  bind:this={slider}
  kind="fader"
  {entry}
  {orientation}
  {label}
  {automated}
  {testid}
  position={positionOf(gainDb)}
  home={UNITY_POSITION}
  valueText={`${formatGain(gainDb)} dB`}
  defaultText={`${formatGain(0)} dB`}
  {onposition}
  {onreset}
  onbegin={gesture.begin}
  onend={gesture.end}
/>
