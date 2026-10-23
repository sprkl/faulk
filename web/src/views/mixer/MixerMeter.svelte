<script lang="ts">
  /**
   * A strip's post-fader meter: two bars (left, right) drawn from `transform`, so a report at 30 Hz
   * costs no layout, a marker that holds the last peak for a moment, a 0 dB mark, and under it the
   * highest peak since the last click (full scale is shown with hatching and a word, not by colour
   * alone). It draws what `ballistics.ts` hands it and holds no state.
   */
  import { fillOf } from './ballistics'
  import type { MeterSnap } from './ballistics'

  let {
    snap,
    label,
    peakLabel,
    clipLabel,
    onreset,
  }: { snap: MeterSnap; label: string; peakLabel: string; clipLabel: string; onreset: () => void } = $props()

  const MARKS = [0, -6, -12, -24, -48]
  const peakText = $derived(snap.peakDb <= -60 ? '−∞' : `${snap.peakDb < 0 ? '−' : '+'}${Math.abs(snap.peakDb).toFixed(1)}`)
</script>

<div class="wrap">
  <div
    class="meter"
    role="meter"
    aria-label={label}
    aria-valuemin={-60}
    aria-valuemax={6}
    aria-valuenow={Math.round(snap.levelDb)}
    data-testid="mixer-meter"
  >
    <div class="bar" style:transform={`scaleY(${snap.levelL})`}></div>
    <div class="bar" style:transform={`scaleY(${snap.levelR})`}></div>
    <div class="hold" style:--hold={snap.holdL}></div>
    <div class="hold r" style:--hold={snap.holdR}></div>
    {#each MARKS as db (db)}
      <span class="mark" class:zero={db === 0} style:--at={fillOf(db)}></span>
    {/each}
  </div>
  <button class="peak" class:clip={snap.clipped} type="button" title={peakLabel} aria-label={peakLabel} data-testid="mixer-peak" onclick={onreset}>
    {#if snap.clipped}{clipLabel}{:else}{peakText}{/if}
  </button>
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--fk-space-1);
  }
  .meter {
    --meter-h: 168px;
    position: relative;
    display: flex;
    gap: 1px;
    width: 14px;
    height: var(--meter-h);
    background: var(--fk-surface-2);
    border-radius: var(--fk-radius-work);
    overflow: hidden;
  }
  .bar {
    flex: 1;
    background: var(--fk-play);
    transform-origin: bottom;
    transform: scaleY(0);
    will-change: transform;
  }
  .hold {
    position: absolute;
    left: 0;
    width: 50%;
    height: 2px;
    bottom: 0;
    background: var(--fk-text);
    transform: translateY(calc(var(--meter-h) * var(--hold, 0) * -1));
    will-change: transform;
  }
  .hold.r {
    left: 50%;
  }
  .mark {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(var(--at) * 100%);
    height: 1px;
    background: var(--fk-bg);
    opacity: 0.7;
    pointer-events: none;
  }
  .mark.zero {
    background: var(--fk-rec);
    opacity: 1;
  }
  .peak {
    min-width: 5ch;
    height: var(--fk-row);
    padding: 0 var(--fk-space-1);
    border: 1px solid var(--fk-line);
    border-radius: var(--fk-radius-work);
    background: transparent;
    color: var(--fk-text-muted);
    font-family: var(--fk-font-mono);
    font-size: 11px;
    cursor: pointer;
  }
  .peak.clip {
    border-color: var(--fk-rec);
    background: repeating-linear-gradient(135deg, var(--fk-rec) 0 4px, var(--fk-bg) 4px 7px);
    color: var(--fk-text);
    font-weight: 700;
  }
</style>
