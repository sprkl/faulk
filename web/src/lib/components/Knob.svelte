<script lang="ts">
  /**
   * A rotary knob drawn over a `Slider` (the behaviour every continuous control shares: drag with a
   * magnet on the default value and a fine mode, double click and context menu to reset, keyboard,
   * `role="slider"`). `position` is 0–1; `display` is the value as text; `home` is the default
   * value's position.
   */
  import { useStores } from '../stores'
  import Slider from './Slider.svelte'

  let {
    label,
    position,
    display,
    home,
    defaultDisplay,
    onchange,
    onreset,
    entry,
    hint,
    automated,
    onbegin,
    onend,
  }: {
    label: string
    position: number
    display: string
    /** The default value's position, held by the magnet. */
    home: number
    /** The default value as text, for the context menu. */
    defaultDisplay: string
    onchange: (position: number) => void
    /** Puts the default value back, as one command. */
    onreset: () => void
    /** Typing a value (`Slider`): Enter on the knob and a click on the value readout open the field. */
    entry?: { text: () => string; commit: (text: string) => boolean }
    /** Tooltip: what the knob does. */
    hint?: string
    /** The tooltip of a knob that an automation lane drives (#388): it marks the knob. */
    automated?: string
    /** A drag starts and ends: the owner groups it as one history entry. */
    onbegin?: () => void
    onend?: () => void
  } = $props()

  const { ui } = useStores()
  let slider: ReturnType<typeof Slider> | undefined = $state()

  const START = -135
  const SWEEP = 270
  const angle = $derived(START + SWEEP * position)

  function arc(from: number, to: number) {
    const r = 18
    const point = (deg: number) => {
      const rad = ((deg - 90) * Math.PI) / 180
      return `${24 + r * Math.cos(rad)} ${24 + r * Math.sin(rad)}`
    }
    const large = to - from > 180 ? 1 : 0
    return `M ${point(from)} A ${r} ${r} 0 ${large} 1 ${point(to)}`
  }
</script>

<div class="knob" class:automated={automated !== undefined} title={automated ?? hint}>
  <span class="label">{label}</span>
  <Slider
    bind:this={slider}
    kind="knob"
    {entry}
    {label}
    {position}
    {home}
    valueText={display}
    defaultText={defaultDisplay}
    {hint}
    {automated}
    {onbegin}
    {onend}
    onposition={onchange}
    {onreset}
  >
    <svg viewBox="0 0 48 48" width="48" height="48" aria-hidden="true">
      <path d={arc(START, START + SWEEP)} class="track" />
      {#if position > 0.001}
        <path d={arc(START, angle)} class="fill" />
      {/if}
      <line x1="24" y1="24" x2="24" y2="10" class="needle" transform={`rotate(${angle} 24 24)`} />
    </svg>
  </Slider>
  {#if entry}
    <button
      class="value"
      type="button"
      tabindex="-1"
      title={ui.t('controls.type')}
      data-testid="value-readout"
      onmousedown={(e) => e.preventDefault()}
      onclick={() => slider?.edit()}>{display}</button
    >
  {:else}
    <span class="value">{display}</span>
  {/if}
</div>

<style>
  .knob {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--fk-space-1);
    min-width: 64px;
  }
  button.value {
    border: 0;
    padding: 0 var(--fk-space-1);
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: text;
  }
  .automated .value {
    color: var(--fk-accent-fg);
  }
  button.value:hover {
    background: var(--fk-surface-2);
  }
  .track {
    fill: none;
    stroke: var(--fk-line);
    stroke-width: 4;
    stroke-linecap: round;
  }
  .fill {
    fill: none;
    stroke: var(--fk-accent-fg);
    stroke-width: 4;
    stroke-linecap: round;
  }
  .needle {
    stroke: var(--fk-text);
    stroke-width: 2;
    stroke-linecap: round;
  }
</style>
