<script lang="ts">
  // An empty view or panel: its name and that it is not built yet.
  import type { MessageKey } from '../../lib/i18n/index'
  import { useStores } from '../../lib/stores'

  let { title }: { title: MessageKey } = $props()
  const { ui } = useStores()
</script>

<div class="placeholder" data-testid="placeholder">
  <h2 class="label">{ui.t(title)}</h2>
  <p>{ui.t('layout.soon')}</p>
</div>

<style>
  .placeholder {
    height: 100%;
    min-height: 96px;
    padding: var(--fk-space-4);
    display: grid;
    place-content: center;
    justify-items: center;
    gap: var(--fk-space-2);
    color: var(--fk-text-muted);
  }
  h2,
  p {
    margin: 0;
  }
</style>
