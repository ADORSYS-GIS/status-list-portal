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

/**
 * When `expiryText` or `expiresSoon` next reads differently, in milliseconds since the epoch:
 * the start of the warning, the next whole day off the time left, or the expiry itself.
 * `null` when they never will.
 */
export function nextExpiryChange(
  issuer: Pick<Issuer, 'expires_at' | 'expires_soon_from'>,
  now = new Date(),
): number | null {
  if (!issuer.expires_at) {
    return null;
  }
  const time = now.getTime();
  const expiry = new Date(issuer.expires_at).getTime();
  const days = Math.ceil((expiry - time) / DAY);
  if (days <= 0) {
    return null;
  }
  // The time left drops to `days - 1`, or the issuer expires when that is 0.
  const changes = [expiry - (days - 1) * DAY];
  if (issuer.expires_soon_from) {
    const warning = new Date(issuer.expires_soon_from).getTime();
    if (warning > time) {
      changes.push(warning);
    }
  }
  return Math.min(...changes);
}

/** How much of a limit is used: `3 of 10`, or the count alone when there is no limit. */
export function usageText(used: number, limit: number | null): string {
  return limit === null ? `${used}` : `${used} of ${limit}`;
}
