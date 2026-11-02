<script lang="ts">
  import { fill } from '../i18n/lang'
  import type { JourneyKey } from '../i18n/journey'
  import { audioUrl } from './entries'
  import { clock } from './format'
  import type { JourneyEntry } from './types'

  /**
   * One track's player. Prerendered with the browser's own audio controls, which work without
   * JavaScript; `live/index.ts` replaces them with the controls below (`hidden` until then). The
   * data attributes are that module's contract.
   */
  let { entry, j, stage, size = 'large' }: { entry: JourneyEntry; j: (key: JourneyKey) => string; stage: string; size?: 'large' | 'small' } = $props()

  const title = $derived(entry.title)
  const total = $derived(clock(entry.duration))
</script>

<div class="player {size}" data-fk-player data-fk-stage-id={stage} data-state="idle" role="group" aria-label={fill(j('player.label'), { title })}>
  <audio data-fk-audio class="native" controls preload="none" crossorigin="anonymous" src={audioUrl(entry)}></audio>
  <div class="controls" data-fk-controls hidden>
    <button
      type="button"
      class="play"
      data-fk-play
      aria-label={fill(j('player.play'), { title })}
      data-label-play={fill(j('player.play'), { title })}
      data-label-pause={fill(j('player.pause'), { title })}
    >
      <svg class="icon icon-play" viewBox="0 0 24 24" aria-hidden="true"
        ><path d="M8 5.5v13a.6.6 0 0 0 .92.5l10.2-6.5a.6.6 0 0 0 0-1L8.92 5a.6.6 0 0 0-.92.5Z" /></svg
      >
      <svg class="icon icon-pause" viewBox="0 0 24 24" aria-hidden="true"
        ><rect x="6.5" y="5" width="4" height="14" rx="1" /><rect x="13.5" y="5" width="4" height="14" rx="1" /></svg
      >
    </button>
    <input
      class="seek"
      type="range"
      min="0"
      max={entry.duration}
      step="1"
      value="0"
      data-fk-seek
      aria-label={fill(j('player.seek'), { title })}
      data-valuetext={j('player.valuetext')}
      aria-valuetext={fill(j('player.valuetext'), { elapsed: '0:00', total })}
    />
    <span class="time" aria-hidden="true"><span data-fk-elapsed>{clock(0)}</span><span class="sep">/</span><span data-fk-total>{total}</span></span>
  </div>
  <p class="error" data-fk-error role="alert" hidden>{j('player.error')} <a href={audioUrl(entry)} rel="external">{j('player.download')}</a></p>
</div>

<style>
  /* `--player-played`: the played part of the slider; `--player-ground`: the surface under the thumb. */
  .player {
    --player-played: var(--fk-accent);
    --player-ground: var(--fk-bg);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .native {
    width: 100%;
    height: 40px;
  }
  .native:global(.is-enhanced) {
    display: none;
  }
  .controls {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 16px;
  }
  .controls[hidden] {
    display: none;
  }
  .play {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    padding: 0;
    border: 0;
    border-radius: var(--fk-radius-pill);
    background: var(--fk-accent);
    color: #fff;
    cursor: pointer;
    transition:
      background-color 160ms,
      transform 160ms cubic-bezier(0.2, 0.7, 0, 1);
  }
  .small .play {
    width: 44px;
    height: 44px;
  }
  .play:hover {
    background: var(--fk-accent-strong);
  }
  .play:active {
    transform: scale(0.94);
  }
  .play:focus-visible,
  .seek:focus-visible {
    outline: 2px solid var(--player-played);
    outline-offset: 3px;
  }
  .play:disabled {
    background: var(--fk-surface-2);
    color: var(--fk-text-muted);
    cursor: not-allowed;
  }
  .icon {
    width: 24px;
    height: 24px;
    fill: currentColor;
  }
  .small .icon {
    width: 20px;
    height: 20px;
  }
  .icon-pause,
  .player:global([data-state='playing']) .icon-play {
    display: none;
  }
  .player:global([data-state='playing']) .icon-pause {
    display: block;
  }
  /* The slider: a hairline track in ink, the played part in ultraviolet, a round thumb. */
  .seek {
    --played: 0%;
    width: 100%;
    height: 24px;
    margin: 0;
    background: transparent;
    appearance: none;
    cursor: pointer;
  }
  .seek::-webkit-slider-runnable-track {
    height: 3px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--player-played) var(--played), currentColor var(--played));
  }
  .seek::-moz-range-track {
    height: 3px;
    border-radius: 2px;
    background: currentColor;
  }
  .seek::-moz-range-progress {
    height: 3px;
    border-radius: 2px;
    background: var(--player-played);
  }
  .seek::-webkit-slider-thumb {
    appearance: none;
    width: 16px;
    height: 16px;
    margin-top: -6.5px;
    border-radius: 50%;
    border: 2px solid var(--player-ground);
    background: var(--player-played);
  }
  .seek::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid var(--player-ground);
    background: var(--player-played);
  }
  .seek:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }
  .time {
    font-family: var(--fk-font-mono);
    font-size: 13px;
    font-feature-settings: 'tnum' 1;
    letter-spacing: 0.02em;
    white-space: nowrap;
  }
  .sep {
    margin: 0 6px;
    color: var(--fk-text-muted);
  }
  .error {
    margin: 0;
    font-size: 14px;
    color: var(--fk-text-muted);
  }
</style>
