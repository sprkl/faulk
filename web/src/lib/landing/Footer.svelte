<script lang="ts">
  import { resolve } from '$app/paths'
  import type { Lang } from '../i18n/lang'
  import LangSwitch from './LangSwitch.svelte'
  import type { Text } from './types'

  /** `page`: as for `Nav`, the section links lead back to the landing page from the Journey page. */
  let { t, lang, page = 'home', langHref: pageLangHref }: { t: Text; lang: Lang; page?: 'home' | 'journey'; langHref?: (l: Lang) => string } = $props()

  const home = $derived(resolve('/[lang=lang]', { lang }))
  const section = (id: string) => (page === 'home' ? `#${id}` : `${home}#${id}`)
  /** The language switch leads to this page in the other language: `langHref`, else the landing page. */
  const langHref = (l: Lang) => pageLangHref?.(l) ?? resolve('/[lang=lang]', { lang: l })
  const SECTIONS = [
    ['modes', 'nav.modes'],
    ['boost', 'nav.boost'],
    ['modules', 'nav.modules'],
  ] as const
</script>

<footer class="lp-wrap footer">
  <div class="row">
    <nav aria-label={t('footer.label')}>
      {#each SECTIONS as [id, label] (id)}
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a fragment, or the resolved landing page plus a fragment -->
        <a href={section(id)}>{t(label)}</a>
      {/each}
      <a href={resolve('/[lang=lang]/journey', { lang })}>{t('nav.journey')}</a>
      <a href={resolve('/app')} data-sveltekit-reload>{t('nav.open')}</a>
    </nav>
    <div class="right">
      <LangSwitch {t} {lang} href={langHref} />
      <span class="lp-mono site">{t('footer.site')}</span>
    </div>
  </div>
  <div class="lp-display wordmark lp-reveal" aria-hidden="true">FAULK</div>
</footer>

<style>
  .footer {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding-top: 48px;
    padding-bottom: 32px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 16px;
    font-size: 14px;
    /* Above the wordmark: its text box reaches up over the links (line-height under 1), and it paints later. */
    position: relative;
    z-index: 1;
  }
  nav {
    display: flex;
    flex-wrap: wrap;
    gap: 22px;
  }
  nav a {
    text-decoration: none;
  }
  .right {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 16px;
  }
  .site {
    font-size: 12px;
    color: var(--fk-text-muted);
  }
  .wordmark {
    font-size: clamp(48px, 21vw, 340px);
    line-height: 0.78;
    letter-spacing: -0.05em;
    /* Decoration only: never catches a click meant for the links above it (#587). */
    pointer-events: none;
    user-select: none;
  }
</style>
