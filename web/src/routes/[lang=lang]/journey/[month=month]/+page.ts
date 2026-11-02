import { error } from '@sveltejs/kit'
import type { Lang } from '../../../../lib/i18n/lang'
import { LANGS } from '../../../../lib/i18n/lang'
import { MONTHS } from '../../../../lib/journey/entries'
import type { EntryGenerator, PageLoad } from './$types'

// A monthly archive of the Journey, in each language.
export const csr = false

export const entries: EntryGenerator = () => LANGS.flatMap((lang) => MONTHS.map((month) => ({ lang, month })))

export const load: PageLoad = ({ params }) => {
  if (!MONTHS.includes(params.month)) error(404)
  return { lang: params.lang as Lang, month: params.month }
}
