import { error } from '@sveltejs/kit';

import { createApi, errorMessage } from '#lib/api/index.ts';
import { requireUser, SESSION_ENDED } from '#lib/session.ts';

import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, parent }) => {
  await requireUser(parent);
  const api = createApi(fetch);
  // An account holds a handful of issuers at most, which the first page always covers.
  const [issuers, registrations] = await Promise.all([
    api.GET('/api/v1/issuers'),
    api.GET('/api/v1/registrations'),
  ]);
  if (issuers.response.status === 401) {
    error(401, SESSION_ENDED);
  }
  if (issuers.error) {
    error(issuers.response.status, errorMessage(issuers.error));
  }
  if (registrations.error) {
    error(registrations.response.status, errorMessage(registrations.error));
  }
  return {
    issuers: issuers.data.issuers,
    registrations: registrations.data.registrations,
  };
};
