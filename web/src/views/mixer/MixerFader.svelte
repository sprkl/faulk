<script lang="ts">
  /**
   * A vertical fader in dB, on the taper of `fader.ts`, with its marks (0 dB is the one drawn
   * strong). The control itself is the shared `GainControl` (magnet on 0 dB, fine mode, reset,
   * keyboard); this draws the marks beside it.
   */
  import GainControl from '../../lib/components/GainControl.svelte'
  import type { Command } from '../../lib/generated/protocol'
  import { FADER_MARKS, positionOf } from '../../lib/project/fader'

  let {
    gainDb,
    label,
    set,
    automated,
    testid,
  }: { gainDb: number | null | undefined; label: string; set: (gainDb: number | null) => Command; automated?: string; testid: string } = $props()

  let control: ReturnType<typeof GainControl> | undefined = $state()

  /** Opens the field to type the gain (the readout under the strip calls it). */
  export function edit() {
    control?.edit()
  }
</script>

<div class="fader">
  <GainControl bind:this={control} {gainDb} {label} {set} {automated} {testid} orientation="vertical" />
  <div class="marks" aria-hidden="true">
    {#each FADER_MARKS as db (db)}
      <span class="mark" class:zero={db === 0} style:--at={positionOf(db)}>{db > 0 ? `+${db}` : db < 0 ? `−${-db}` : '0'}</span>
    {/each}
  </div>
</div>

<style>
  .fader {
    position: relative;
    display: flex;
    align-items: stretch;
    gap: var(--fk-space-1);
    height: 168px;
  }
  .marks {
    position: relative;
    width: 3ch;
  }
  .mark {
    position: absolute;
    left: 0;
    bottom: calc(var(--at) * (100% - 16px) + 8px);
    transform: translateY(50%);
    font-family: var(--fk-font-mono);
    font-size: 9px;
    line-height: 1;
    color: var(--fk-text-muted);
  }
  .mark.zero {
    color: var(--fk-text);
    font-weight: 700;
  }
  .mark.zero::before {
    content: '';
    position: absolute;
    right: 100%;
    top: 50%;
    width: 6px;
    height: 2px;
    margin-right: 2px;
    background: var(--fk-text);
  }
</style>
