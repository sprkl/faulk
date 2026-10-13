/**
 * The landing page ships no client-side JavaScript bundle (`csr = false`); these two inline
 * scripts are its only code. They are built from `defaultLang`'s own source, so the rule the
 * DAW uses and the rule the landing page uses are one function.
 */

import { defaultLang, LANG_STORAGE_KEY } from '../i18n/lang'

/** JSON for a `<script>` body: `<` is escaped so a value can never close the tag. */
function literal(value: string): string {
  return JSON.stringify(value).replace(/</g, '\\u003c')
}

/**
 * For `/` only: moves the reader to the French page when the stored choice, or else the
 * browser's language, is French. `/en` and `/fr` never redirect, so no loop is possible.
 */
export function detectScript(frenchHref: string): string {
  return (
    `(function(){var s=null;try{s=localStorage.getItem(${literal(LANG_STORAGE_KEY)})}catch(e){}` +
    `var pick=${defaultLang.toString()};` +
    `if(pick(s,navigator.language)==='fr')location.replace(${literal(frenchHref)})})()`
  )
}

/** Stores the language of a clicked `a[data-lang]` link, shared with the DAW. */
export function rememberScript(): string {
  return (
    `document.addEventListener('click',function(e){var t=e.target;` +
    `var a=t&&t.closest?t.closest('a[data-lang]'):null;if(!a)return;` +
    `try{localStorage.setItem(${literal(LANG_STORAGE_KEY)},a.getAttribute('data-lang'))}catch(_){}})`
  )
}

/** A `<script>` element with the given body, for `{@html}` in `<svelte:head>`. */
export function inlineScript(body: string): string {
  return `<script>${body}</` + `script>`
}
