import type { Lang } from '../../lib/i18n/lang'
import type { EntryGenerator, PageLoad } from './$types'

// Plain HTML, like `/`.
export const csr = false

export const entries: EntryGenerator = () => [{ lang: 'en' }, { lang: 'fr' }]

export const load: PageLoad = ({ params }) => ({ lang: params.lang as Lang })
