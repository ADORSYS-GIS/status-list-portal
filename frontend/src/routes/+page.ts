import { redirect } from '@sveltejs/kit';

import { signInError } from '#lib/sign-in.ts';

import type { PageLoad } from './$types';

export const load: PageLoad = async ({ parent, url }) => {
  const { user } = await parent();
  if (user) {
    redirect(307, '/issuers');
  }
  return { signInError: signInError(url.searchParams.get('sign_in_error')) };
};
