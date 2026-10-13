<script lang="ts">
  import { resolve } from '$app/paths'
  import type { Lang } from '../i18n/lang'
  import LangSwitch from './LangSwitch.svelte'
  import type { Text } from './types'

  let { t, lang }: { t: Text; lang: Lang } = $props()
</script>

<footer class="lp-wrap footer">
  <div class="row">
    <nav aria-label={t('footer.label')}>
      <a href="#modes">{t('nav.modes')}</a>
      <a href="#boost">{t('nav.boost')}</a>
      <a href="#modules">{t('nav.modules')}</a>
      <a href={resolve('/app')} data-sveltekit-reload>{t('nav.open')}</a>
    </nav>
    <div class="right">
      <LangSwitch {t} {lang} />
      <span class="lp-mono site">{t('footer.site')}</span>
    </div>
  </div>
  <div class="lp-display wordmark" aria-hidden="true">FAULK</div>
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
  }
</style>
