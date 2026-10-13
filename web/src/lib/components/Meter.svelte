<script lang="ts">
  /** Peak meter in dBFS; turns to the clip style (red + hatching + label) at 0 dBFS. */
  let { peak, label, clipLabel }: { peak: number; label: string; clipLabel: string } = $props()

  const FLOOR_DB = -60
  const db = $derived(peak > 0 ? 20 * Math.log10(peak) : -Infinity)
  const fill = $derived(Math.max(0, Math.min(1, (db - FLOOR_DB) / -FLOOR_DB)))
  const clip = $derived(peak >= 0.999)
</script>

<div class="meter" role="meter" aria-label={label} aria-valuemin={FLOOR_DB} aria-valuemax={0} aria-valuenow={Number.isFinite(db) ? Math.round(db) : FLOOR_DB}>
  <div class="bar" class:clip style:height={`${fill * 100}%`}></div>
  {#if clip}<span class="clip-label">{clipLabel}</span>{/if}
</div>
<span class="value db">{Number.isFinite(db) ? db.toFixed(1) : '−∞'} dB</span>

<style>
  .meter {
    position: relative;
    width: 12px;
    height: 120px;
    background: var(--fk-surface-2);
    border-radius: var(--fk-radius-work);
    overflow: hidden;
  }
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--fk-play);
  }
  .bar.clip {
    background: repeating-linear-gradient(135deg, var(--fk-rec) 0 4px, var(--fk-bg) 4px 7px);
  }
  .clip-label {
    position: absolute;
    top: 2px;
    left: 50%;
    transform: translateX(-50%) rotate(-90deg);
    font-size: 8px;
    font-weight: 700;
    color: var(--fk-rec);
  }
  .db {
    color: var(--fk-text-muted);
  }
</style>
