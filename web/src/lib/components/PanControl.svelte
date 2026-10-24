<script lang="ts">
  /**
   * The pan of a track, from -1 (left) to 1 (right), centred by default, shared by the mixer and the
   * track list. A `Slider`: magnet on the centre, fine mode, reset to the centre as one command. One
   * drag is one history group (`gesture.ts`). It holds no project state.
   */
  import { onDestroy } from 'svelte'
  import { createGesture } from '../gesture'
  import { formatPan } from '../project/tracks'
  import { useStores } from '../stores'
  import { setPan } from '../track-commands'
  import Slider from './Slider.svelte'

  let { track, pan, label, testid }: { track: string; pan: number; label: string; testid?: string } = $props()

  const { dispatcher } = useStores()
  const gesture = createGesture((command) => dispatcher.run(command))
  onDestroy(() => gesture.end())

  function onposition(position: number) {
    // Three decimals: the fine mode goes below the percent the readout shows.
    const next = position === 0.5 ? 0 : Math.round((position * 2 - 1) * 1000) / 1000
    if (next !== pan) dispatcher.run(setPan(track, next))
  }

  function onreset() {
    // Already centred: no command, no empty history entry.
    if (pan !== 0) dispatcher.run(setPan(track, 0))
  }
</script>

<Slider
  kind="pan"
  {label}
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
