<script lang="ts">
  // The track list column; tracks and their headers arrive with UI-2.
  import { useStores } from '../../lib/stores'

  const { ui } = useStores()
</script>

<aside class="tracks" aria-labelledby="fk-tracks-title" data-testid="track-list">
  <h2 id="fk-tracks-title" class="label">{ui.t('tracks.title')}</h2>
  <p class="empty">{ui.t('tracks.empty')}</p>
</aside>

<style>
  .tracks {
    grid-area: tracks;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: var(--fk-space-3);
    padding: var(--fk-space-3);
    border-right: 1px solid var(--fk-line);
    background: var(--fk-surface);
  }
  h2,
  p {
    margin: 0;
  }
  .empty {
    color: var(--fk-text-muted);
  }
  @media (max-width: 760px) {
    .tracks {
      border-right: 0;
      border-top: 1px solid var(--fk-line);
    }
  }
</style>
