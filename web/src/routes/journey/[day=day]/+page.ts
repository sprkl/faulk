import { error } from '@sveltejs/kit'
import { ENTRIES, entryByFolder, folderOf } from '../../../lib/journey/entries'
import type { EntryGenerator, PageLoad } from './$types'

// One day's page of the Journey, in English (French readers move to `/fr/journey/<day>`).
export const csr = false

export const entries: EntryGenerator = () => ENTRIES.map((e) => ({ day: folderOf(e) }))

export const load: PageLoad = ({ params }) => {
  if (!entryByFolder(params.day)) error(404)
  return { day: params.day }
}
