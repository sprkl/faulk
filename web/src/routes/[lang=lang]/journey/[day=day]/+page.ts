import { error } from '@sveltejs/kit'
import type { Lang } from '../../../../lib/i18n/lang'
import { LANGS } from '../../../../lib/i18n/lang'
import { ENTRIES, entryByFolder, folderOf } from '../../../../lib/journey/entries'
import type { EntryGenerator, PageLoad } from './$types'

// One day's page of the Journey, in each language.
export const csr = false

export const entries: EntryGenerator = () => LANGS.flatMap((lang) => ENTRIES.map((e) => ({ lang, day: folderOf(e) })))

export const load: PageLoad = ({ params }) => {
  if (!entryByFolder(params.day)) error(404)
  return { lang: params.lang as Lang, day: params.day }
}
