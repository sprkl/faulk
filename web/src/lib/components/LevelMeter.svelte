<script lang="ts">
  /**
   * The one level meter of the app (DESIGN.md > Meter): two channels of flat segments up to -12 dB,
   * then wider segments, so the zone reads by shape and not by colour; a peak hold line (the
   * ballistics hold it 1.5 s); and a latched CLIP, a hatched block with the word, that stays until
   * it is clicked or activated from the keyboard. It draws what `meters/ballistics.ts` hands it and
   * holds no state: the bars move by `transform`, so a report at 30 Hz costs no layout.
   *
   * `vertical` is the mixer's and the instrument view's; `horizontal` fits a lane header and the top bar.
   * `readout` adds the highest peak under (or beside) the bars, which is also the reset button.
   */
  import { fillOf, METER_FLOOR_DB } from '../meters/ballistics'
  import type { MeterSnap } from '../meters/ballistics'

  let {
    snap,
    label,
    clipLabel,
    resetLabel,
    onreset,
    orientation = 'vertical',
    readout = false,
    testid = 'level-meter',
  }: {
    snap: MeterSnap
    /** The meter's accessible name. */
    label: string
    /** The word shown on a latched clip. */
    clipLabel: string
    /** What the reset button does, for its name and tooltip. */
    resetLabel: string
    onreset: () => void
    orientation?: 'vertical' | 'horizontal'
    readout?: boolean
    testid?: string
  } = $props()

  const MARKS = [0, -6, -12, -24, -48]
  /** Where the wide zone starts, as a share of the meter's length. */
  const SPLIT = fillOf(-12)
  const peakText = $derived(snap.peakDb <= METER_FLOOR_DB ? '−∞' : `${snap.peakDb < 0 ? '−' : '+'}${Math.abs(snap.peakDb).toFixed(1)}`)

  /** The share (0..1) of the flat zone a level fills, and of the wide zone. */
  const low = (level: number) => Math.min(1, level / SPLIT)
  const high = (level: number) => Math.max(0, (level - SPLIT) / (1 - SPLIT))
</script>

<div class="wrap {orientation}" class:with-readout={readout}>
  <div
    class="meter"
    role="meter"
    aria-label={label}
    aria-valuemin={METER_FLOOR_DB}
    aria-valuemax={6}
    aria-valuenow={Math.round(snap.levelDb)}
    aria-valuetext={snap.clipped ? clipLabel : `${Math.round(snap.levelDb)} dB`}
    data-testid={testid}
    data-clipped={snap.clipped ? 'true' : undefined}
    style:--split="{SPLIT * 100}%"
  >
    {#each [[snap.levelL, snap.holdL], [snap.levelR, snap.holdR]] as [level, hold], i (i)}
      <div class="channel">
        <div class="zone flat"><span class="fill" style:--f={low(level!)}></span></div>
        <div class="zone wide"><span class="fill" style:--f={high(level!)}></span></div>
        <span class="hold" style:--at={hold}></span>
      </div>
    {/each}
    {#each MARKS as db (db)}
      <span class="mark" class:zero={db === 0} style:--at={fillOf(db)}></span>
    {/each}
    {#if snap.clipped && !readout}
      <!-- A latched clip: the hatched block with the word covers the meter and resets it; it is the only way to reset a meter without a readout. -->
      <button class="clip-block" type="button" title={resetLabel} aria-label={`${clipLabel}. ${resetLabel}`} data-testid="{testid}-clip" onclick={onreset}>
        {clipLabel}<span aria-hidden="true"> ×</span>
      </button>
    {/if}
  </div>
  {#if readout}
    <button class="peak" class:clip={snap.clipped} type="button" title={resetLabel} aria-label={resetLabel} data-testid="{testid}-peak" onclick={onreset}>
      {#if snap.clipped}{clipLabel}<span aria-hidden="true"> ×</span>{:else}{peakText}{/if}
    </button>
  {/if}
</div>

<style>
  .wrap {
    display: flex;
    align-items: center;
    gap: var(--fk-space-1);
  }
  .wrap.vertical {
    flex-direction: column;
  }
  .meter {
    position: relative;
    display: flex;
    gap: 1px;
    overflow: hidden;
    background: var(--fk-surface-2);
    border-radius: var(--fk-radius-work);
  }
  .vertical .meter {
    --len: 168px;
    width: 14px;
    height: var(--len);
  }
  .horizontal .meter {
    --len: 72px;
    flex-direction: column;
    width: var(--len);
    height: 14px;
  }
  .channel {
    position: relative;
    flex: 1;
    display: flex;
    min-width: 0;
    min-height: 0;
  }
  .vertical .channel {
    flex-direction: column-reverse;
    align-items: center;
  }
  .horizontal .channel {
    flex-direction: row;
    align-items: center;
  }
  /* The flat zone (floor to -12 dB) is thinner than the wide one (-12 dB and above): the zone reads by shape. */
  .zone {
    position: relative;
    height: 100%;
    width: 100%;
  }
  .vertical .zone.flat {
    height: var(--split);
    width: 60%;
  }
  .vertical .zone.wide {
    height: calc(100% - var(--split));
  }
  .horizontal .zone.flat {
    width: var(--split);
    height: 60%;
  }
  .horizontal .zone.wide {
    width: calc(100% - var(--split));
  }
  .fill {
    position: absolute;
    inset: 0;
    background: var(--fk-play);
    will-change: transform;
  }
  .vertical .fill {
    transform-origin: bottom;
    transform: scaleY(var(--f, 0));
  }
  .horizontal .fill {
    transform-origin: left;
    transform: scaleX(var(--f, 0));
  }
  /* The segments: a 1px gap every 3px, cut out of the bar. */
  .vertical .zone {
    mask-image: repeating-linear-gradient(to top, var(--fk-text) 0 2px, transparent 2px 3px);
  }
  .horizontal .zone {
    mask-image: repeating-linear-gradient(to right, var(--fk-text) 0 2px, transparent 2px 3px);
  }
  .hold {
    position: absolute;
    background: var(--fk-text);
    will-change: transform;
  }
  .vertical .hold {
    left: 0;
    right: 0;
    bottom: 0;
    height: 2px;
    transform: translateY(calc(var(--len) * var(--at, 0) * -1));
  }
  .horizontal .hold {
    top: 0;
    bottom: 0;
    left: 0;
    width: 2px;
    transform: translateX(calc(var(--len) * var(--at, 0)));
  }
  .mark {
    position: absolute;
    background: var(--fk-bg);
    opacity: 0.7;
    pointer-events: none;
  }
  .vertical .mark {
    left: 0;
    right: 0;
    bottom: calc(var(--at) * 100%);
    height: 1px;
  }
  .horizontal .mark {
    top: 0;
    bottom: 0;
    left: calc(var(--at) * 100%);
    width: 1px;
  }
  .mark.zero {
    background: var(--fk-rec);
    opacity: 1;
  }
  .clip-block,
  .peak.clip {
    border: 1px solid var(--fk-rec);
    background: repeating-linear-gradient(135deg, var(--fk-rec) 0 4px, var(--fk-bg) 4px 7px);
    color: var(--fk-text);
    font-family: var(--fk-font-mono);
    font-weight: 700;
    cursor: pointer;
  }
  .clip-block {
    position: absolute;
    inset: 0;
    padding: 0;
    border-radius: var(--fk-radius-work);
    font-size: 9px;
    line-height: 12px;
    letter-spacing: 0.04em;
    text-shadow:
      0 0 2px var(--fk-bg),
      0 0 2px var(--fk-bg),
      0 0 3px var(--fk-bg);
  }
  /* A short horizontal meter (the top bar's, a lane's) cannot hold the word: the latched block grows to its text and sits over the meter's end. */
  .horizontal .meter {
    overflow: visible;
  }
  .horizontal .clip-block {
    inset: 0 0 0 auto;
    width: max-content;
    min-width: 100%;
    padding: 0 4px;
    z-index: 2;
    background-color: var(--fk-bg);
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
    text-shadow:
      0 0 2px var(--fk-bg),
      0 0 2px var(--fk-bg);
  }
</style>
