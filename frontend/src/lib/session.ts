import { redirect } from '@sveltejs/kit';

import type { User } from './api';

/** The signed-in user. Without one, the page sends the browser to the home page. */
export async function requireUser(parent: () => Promise<{ user: User | null }>): Promise<User> {
  const { user } = await parent();
  if (!user) {
    redirect(303, '/');
  }
  return user;
}
