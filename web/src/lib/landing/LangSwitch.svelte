<script lang="ts">
  import { resolve } from '$app/paths'
  import { LANGS, type Lang } from '../i18n/lang'
  import type { Text } from './types'

  /** Real links to `/en` and `/fr` (or `href`'s pages); the inline script stores the choice on click. */
  let { t, lang, href = (l: Lang) => resolve('/[lang=lang]', { lang: l }) }: { t: Text; lang: Lang; href?: (l: Lang) => string } = $props()
</script>

<ul class="switch" aria-label={t('lang.label')}>
  {#each LANGS as l (l)}
    <li>
      <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- `href` returns resolved paths -->
      <a href={href(l)} hreflang={l} lang={l} data-lang={l} aria-current={l === lang ? 'page' : undefined} aria-label={t(l === 'en' ? 'lang.en' : 'lang.fr')}
        >{l.toUpperCase()}</a
      >
    </li>
  {/each}
</ul>

<style>
  .switch {
    display: inline-flex;
    margin: 0;
    padding: 2px;
    list-style: none;
    height: 36px;
    border: 1px solid var(--fk-text);
    border-radius: var(--fk-radius-pill);
    font: 500 12px var(--fk-font-mono);
    letter-spacing: 0.04em;
  }
  li {
    display: flex;
  }
  a {
    display: inline-flex;
    align-items: center;
    padding: 0 12px;
    border-radius: var(--fk-radius-pill);
    text-decoration: none;
  }
  a[aria-current='page'] {
    background: var(--fk-text);
    color: var(--fk-bg);
  }
  a[aria-current='page']:hover {
    color: var(--fk-bg);
  }
</style>
