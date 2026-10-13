import { defineParams } from '@sveltejs/kit/params'
import { isLang } from './lib/i18n/lang.ts'

/** Route parameter matchers: `[lang=lang]` accepts `en` and `fr` only. */
export const params = defineParams({
  lang: (param) => (isLang(param) ? param : undefined),
})
