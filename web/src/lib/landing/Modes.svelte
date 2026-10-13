<script lang="ts">
  import type { LandingKey } from '../i18n/landing'
  import type { Text } from './types'

  let { t }: { t: Text } = $props()

  const MODES = ['web', 'boost', 'atelier'] as const
  const COLUMNS = ['Runs', 'Latency', 'Gain', 'Status'] as const
  const key = (mode: string, field: string) => `modes.${mode}${field}` as LandingKey
</script>

<section id="modes" class="lp-wrap lp-section modes" aria-labelledby="modes-title">
  <h2 id="modes-title" class="lp-display lp-h2">{t('modes.title1')}<br />{t('modes.title2')}</h2>
  <!-- Explicit roles keep the table semantics when the phone layout changes its display (WebKit). -->
  <!-- svelte-ignore a11y_no_redundant_roles -->
  <table role="table">
    <caption class="visually-hidden">{t('modes.caption')}</caption>
    <!-- svelte-ignore a11y_no_redundant_roles -->
    <thead role="rowgroup">
      <!-- svelte-ignore a11y_no_redundant_roles -->
      <tr role="row" class="lp-mono">
        <th role="columnheader" scope="col">{t('modes.colMode')}</th>
        {#each COLUMNS as col (col)}
          <th role="columnheader" scope="col">{t(`modes.col${col}` as LandingKey)}</th>
        {/each}
      </tr>
    </thead>
    <!-- svelte-ignore a11y_no_redundant_roles -->
    <tbody role="rowgroup">
      {#each MODES as mode (mode)}
        <!-- svelte-ignore a11y_no_redundant_roles -->
        <tr role="row">
          <th role="rowheader" scope="row" class="lp-display name" class:accent={mode === 'boost'}>{t(key(mode, 'Name'))}</th>
          {#each COLUMNS as col (col)}
            <td role="cell" class:mono={col === 'Latency' || col === 'Status'} data-label={t(`modes.col${col}` as LandingKey)}>
              {t(key(mode, col))}
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
  </table>
</section>

<style>
  .modes {
    display: flex;
    flex-direction: column;
    gap: 32px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 16px;
  }
  thead th {
    padding: 12px 16px 12px 0;
    border-bottom: 1px solid var(--fk-text);
    font-size: 12px;
    font-weight: 500;
    text-align: left;
    color: var(--fk-text-muted);
  }
  thead th:first-child {
    width: 18%;
  }
  tbody th,
  td {
    padding: 22px 16px 22px 0;
    border-bottom: 1px solid var(--fk-line);
    text-align: left;
    vertical-align: baseline;
  }
  .name {
    font-size: 28px;
  }
  .accent {
    color: var(--fk-accent);
  }
  .mono {
    font-family: var(--fk-font-mono);
    font-size: 14px;
  }
  @media (max-width: 760px) {
    thead {
      position: absolute;
      width: 1px;
      height: 1px;
      overflow: hidden;
      clip: rect(0 0 0 0);
    }
    tbody tr {
      display: grid;
      gap: 8px;
      padding: 20px 0;
      border-bottom: 1px solid var(--fk-line);
    }
    tbody th,
    td {
      display: grid;
      grid-template-columns: 140px minmax(0, 1fr);
      gap: 12px;
      padding: 0;
      border: 0;
    }
    tbody th {
      display: block;
      padding-bottom: 4px;
    }
    td::before {
      content: attr(data-label);
      font-family: var(--fk-font-mono);
      font-size: 12px;
      letter-spacing: 0.04em;
      color: var(--fk-text-muted);
    }
  }
</style>
