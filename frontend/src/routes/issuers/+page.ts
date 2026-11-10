import { error } from '@sveltejs/kit';

import { allIssuers, createApi, errorMessage } from '#lib/api/index.ts';
import { requireUser, SESSION_ENDED } from '#lib/session.ts';

import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, parent }) => {
  await requireUser(parent);
  const api = createApi(fetch);
  const [issuers, registrations] = await Promise.all([
    allIssuers(api),
    api.GET('/api/v1/registrations'),
  ]);
  if ('error' in issuers) {
    error(issuers.status, issuers.status === 401 ? SESSION_ENDED : errorMessage(issuers.error));
  }
  if (registrations.error) {
    error(registrations.response.status, errorMessage(registrations.error));
  }
  return {
    issuers: issuers.issuers,
    registrations: registrations.data.registrations,
  };
};
