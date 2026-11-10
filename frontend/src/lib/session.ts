import { resolve } from '$app/paths';
import { redirect } from '@sveltejs/kit';

import type { User } from './api/index.ts';

/** What a page says when its session ended while the app was open. */
export const SESSION_ENDED = 'Your session has ended. Please sign in again.';

/** The signed-in user. Without one, the page sends the browser to the home page. */
export async function requireUser(parent: () => Promise<{ user: User | null }>): Promise<User> {
  const { user } = await parent();
  if (!user) {
    redirect(303, resolve('/'));
  }
  return user;
}
