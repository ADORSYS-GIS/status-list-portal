import { error } from '@sveltejs/kit';

import { createApi, errorMessage } from '#lib/api/index.ts';

import type { LayoutLoad } from './$types';

// The backend serves one page for every path; the app renders in the browser.
export const ssr = false;

export const load: LayoutLoad = async ({ fetch }) => {
  const api = createApi(fetch);
  const [me, terms] = await Promise.all([api.GET('/api/v1/me'), api.GET('/api/v1/terms')]);
  if (me.error && me.response.status !== 401) {
    error(me.response.status, errorMessage(me.error));
  }
  return { user: me.data ?? null, terms: terms.data ?? null };
};
