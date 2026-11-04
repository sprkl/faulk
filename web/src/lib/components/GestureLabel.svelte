<script lang="ts">
  /**
   * The label near the pointer during a move or a resize (#612): a few short lines (the value, the snap, the key that frees
   * it). Drawn over a positioned parent, at the pointer's place in it; it flips to the other side of the pointer near the
   * parent's right and bottom edges, by its measured size. Sighted help only: the live regions and the cells say what the gesture did.
   */
  interface Props {
    /** The pointer, in the parent's CSS pixels. */
    x: number
    y: number
    /** The parent's size, to keep the label inside it. */
    width: number
    height: number
    lines: readonly string[]
  }
  const { x, y, width, height, lines }: Props = $props()

  const OFFSET = 16
  /** The label's own size, measured (a French line is longer than an English one); a guess until the first measure. */
  let labelW = $state(190)
  let labelH = $state(64)
  const flipped = $derived(x + OFFSET + labelW > width)
  const left = $derived(flipped ? Math.max(0, x - OFFSET - labelW) : x + OFFSET)
  const top = $derived(y + OFFSET + labelH > height ? Math.max(0, y - OFFSET - labelH) : y + OFFSET)
</script>

<div
  class="label"
  style:left="{left}px"
  style:top="{top}px"
  bind:clientWidth={labelW}
  bind:clientHeight={labelH}
  aria-hidden="true"
  data-testid="gesture-label"
>
  {#each lines as line, i (i)}
    <span class:main={i === 0}>{line}</span>
  {/each}
</div>

<style>
  .label {
    position: absolute;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: var(--fk-space-1) var(--fk-space-2);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-panel);
    background: var(--fk-bg);
    color: var(--fk-text-muted);
    box-shadow: var(--fk-shadow-float);
    font-size: 11px;
    line-height: 1.35;
    text-transform: none;
    letter-spacing: normal;
    white-space: nowrap;
    pointer-events: none;
  }
  .main {
    color: var(--fk-text);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
</style>
