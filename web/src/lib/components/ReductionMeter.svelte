<script lang="ts">
  /**
   * The gain a Compressor takes off the signal, in dB: a bar that grows from the left as the gain
   * closes, and the number in mono (the bar alone is not the information). The engine reports the
   * largest reduction of the strip's compressors, so a strip with several shows the deepest.
   */
  let { reductionDb, label, testid = 'gain-reduction' }: { reductionDb: number; label: string; testid?: string } = $props()

  /** The bar's full scale, in dB. */
  const SCALE_DB = 24
  const share = $derived(Math.min(1, Math.max(0, reductionDb / SCALE_DB)))
  const text = $derived(reductionDb < 0.05 ? '0.0' : `−${reductionDb.toFixed(1)}`)
</script>

<div class="gr" data-testid={testid}>
  <span class="name">{label}</span>
  <div class="bar" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={SCALE_DB} aria-valuenow={Math.round(reductionDb)}>
    <span class="fill" style:--f={share}></span>
  </div>
  <span class="value" data-testid="{testid}-value">{text} dB</span>
</div>

<style>
  .gr {
    display: flex;
    align-items: center;
    gap: var(--fk-space-2);
    min-width: 0;
  }
  .name {
    color: var(--fk-text-muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .bar {
    flex: 1;
    min-width: 40px;
    height: 10px;
    overflow: hidden;
    border-radius: var(--fk-radius-work);
    background: var(--fk-surface-2);
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--fk-text-muted);
    transform-origin: left;
    transform: scaleX(var(--f, 0));
    mask-image: repeating-linear-gradient(to right, var(--fk-text) 0 2px, transparent 2px 3px);
  }
  .value {
    min-width: 8ch;
    text-align: right;
  }
</style>
