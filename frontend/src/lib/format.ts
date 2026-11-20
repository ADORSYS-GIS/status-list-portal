import type { Issuer } from '#lib/api/index.ts';

const DAY = 24 * 60 * 60 * 1000;

const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });

/** A date and time in the reader's locale and time zone. */
export function formatDateTime(iso: string): string {
  return dateTime.format(new Date(iso));
}

/** When an issuer stops being active, in words. */
export function expiryText(expiresAt: string | null | undefined, now = new Date()): string {
  if (!expiresAt) {
    return 'Never';
  }
  const expiry = new Date(expiresAt);
  const days = Math.ceil((expiry.getTime() - now.getTime()) / DAY);
  if (days <= 0) {
    return `Expired on ${formatDateTime(expiresAt)}`;
  }
  const remaining = days === 1 ? 'in 1 day' : `in ${days} days`;
  return `${formatDateTime(expiresAt)}, ${remaining}`;
}

/**
 * Whether an active issuer stops being active soon, unless it is renewed: from its
 * `expires_soon_from`, which the backend sets from the sandbox lifetime, until its expiry.
 */
export function expiresSoon(
  issuer: Pick<Issuer, 'state' | 'expires_at' | 'expires_soon_from'>,
  now = new Date(),
): boolean {
  if (issuer.state !== 'active' || !issuer.expires_at || !issuer.expires_soon_from) {
    return false;
  }
  const time = now.getTime();
  return (
    time >= new Date(issuer.expires_soon_from).getTime() &&
    time < new Date(issuer.expires_at).getTime()
  );
}

/** How much of a limit is used: `3 of 10`, or the count alone when there is no limit. */
export function usageText(used: number, limit: number | null): string {
  return limit === null ? `${used}` : `${used} of ${limit}`;
}
