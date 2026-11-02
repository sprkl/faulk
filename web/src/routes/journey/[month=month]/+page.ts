import { error } from '@sveltejs/kit'
import { MONTHS } from '../../../lib/journey/entries'
import type { EntryGenerator, PageLoad } from './$types'

// A monthly archive of the Journey, in English (French readers move to `/fr/journey/<month>`).
export const csr = false

export const entries: EntryGenerator = () => MONTHS.map((month) => ({ month }))

export const load: PageLoad = ({ params }) => {
  if (!MONTHS.includes(params.month)) error(404)
  return { month: params.month }
}
