/** Why the last sign-in failed, as the backend reports it in the home page's `sign_in_error`. */
export type SignInError = 'denied' | 'expired' | 'no_verified_email' | 'unavailable';

export const SIGN_IN_ERRORS: Record<SignInError, string> = {
  denied: 'Signing in was cancelled on GitHub.',
  expired: 'This sign-in expired or was already used. Please sign in again.',
  no_verified_email:
    'Signing in needs a verified primary email on your GitHub account. Add one, then sign in again.',
  unavailable: 'Signing in is not available right now. Please try again later.',
};

/** The failure that `reason` names, if it names one. */
export function signInError(reason: string | null): SignInError | null {
  return reason !== null && Object.hasOwn(SIGN_IN_ERRORS, reason) ? (reason as SignInError) : null;
}
