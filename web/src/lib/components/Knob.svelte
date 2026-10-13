<script lang="ts">
  /**
   * A rotary knob drawn over a real range input, so it works with the keyboard,
   * screen readers and touch. `position` is 0–1; `format` shows the value.
   */
  let {
    label,
    position,
    display,
    onchange,
  }: {
    label: string
    position: number
    display: string
    onchange: (position: number) => void
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

<label class="knob">
  <span class="label">{label}</span>
  <span class="dial">
    <svg viewBox="0 0 48 48" width="48" height="48" aria-hidden="true">
      <path d={arc(START, START + SWEEP)} class="track" />
      {#if position > 0.001}
        <path d={arc(START, angle)} class="fill" />
      {/if}
      <line x1="24" y1="24" x2="24" y2="10" class="needle" transform={`rotate(${angle} 24 24)`} />
    </svg>
    <input
      type="range"
      min="0"
      max="1"
      step="0.001"
      value={position}
      aria-valuetext={display}
      oninput={(e) => onchange(Number(e.currentTarget.value))}
    />
  </span>
  <span class="value">{display}</span>
</label>

<style>
  .knob {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--fk-space-1);
    min-width: 64px;
  }
  .dial {
    position: relative;
    width: 48px;
    height: 48px;
  }
  input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: ew-resize;
  }
  .dial:has(input:focus-visible) svg {
    outline: 2px solid var(--fk-accent-fg);
    outline-offset: 2px;
    border-radius: 50%;
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
