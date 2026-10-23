import { error } from '@sveltejs/kit';

import { createApi, errorMessage } from '#lib/api/index.ts';
import { requireUser, SESSION_ENDED } from '#lib/session.ts';

import type { PageLoad } from './$types';

export const load: PageLoad = async ({ fetch, params, parent }) => {
  await requireUser(parent);
  const issuer = await createApi(fetch).GET('/api/v1/issuers/{id}', {
    params: { path: { id: params.id } },
  });
  if (issuer.response.status === 401) {
    error(401, SESSION_ENDED);
  }
  if (issuer.response.status === 404) {
    error(404, 'You have no issuer with this id.');
  }
  if (issuer.error) {
    error(issuer.response.status, errorMessage(issuer.error));
  }
  return { issuer: issuer.data };
};
