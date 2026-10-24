<script lang="ts">
  /**
   * A rotary knob drawn over a `Slider` (the behaviour every continuous control shares: drag with a
   * magnet on the default value and a fine mode, double click and context menu to reset, keyboard,
   * `role="slider"`). `position` is 0–1; `display` is the value as text; `home` is the default
   * value's position.
   */
  import Slider from './Slider.svelte'

  let {
    label,
    position,
    display,
    home,
    defaultDisplay,
    onchange,
    onreset,
    hint,
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
    /** Tooltip: what the knob does. */
    hint?: string
    /** A drag starts and ends: the owner groups it as one history entry. */
    onbegin?: () => void
    onend?: () => void
  } = $props()

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

<div class="knob" title={hint}>
  <span class="label">{label}</span>
  <Slider kind="knob" {label} {position} {home} valueText={display} defaultText={defaultDisplay} {hint} {onbegin} {onend} onposition={onchange} {onreset}>
    <svg viewBox="0 0 48 48" width="48" height="48" aria-hidden="true">
      <path d={arc(START, START + SWEEP)} class="track" />
      {#if position > 0.001}
        <path d={arc(START, angle)} class="fill" />
      {/if}
      <line x1="24" y1="24" x2="24" y2="10" class="needle" transform={`rotate(${angle} 24 24)`} />
    </svg>
  </Slider>
  <span class="value">{display}</span>
</div>

<style>
  .knob {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--fk-space-1);
    min-width: 64px;
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
