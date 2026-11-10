import { requireUser } from '#lib/session.ts';

import type { PageLoad } from './$types';

export const load: PageLoad = async ({ parent }) => {
  await requireUser(parent);
};
