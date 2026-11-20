import type { Issuer } from '#lib/api/index.ts';

const DAY = 24 * 60 * 60 * 1000;

const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });

/** A date and time in the reader's locale and time zone. */
export function formatDateTime(iso: string): string {
  return dateTime.format(new Date(iso));
}

/** Whole days until `iso`, a part of a day counting as one. Zero or less once it is past. */
function daysUntil(iso: string, now: Date): number {
  return Math.ceil((new Date(iso).getTime() - now.getTime()) / DAY);
}

/** When an issuer stops being active, in words. */
export function expiryText(expiresAt: string | null | undefined, now = new Date()): string {
  if (!expiresAt) {
    return 'Never';
  }
  const days = daysUntil(expiresAt, now);
  if (days <= 0) {
    return `Expired on ${formatDateTime(expiresAt)}`;
  }
  const remaining = days === 1 ? 'in 1 day' : `in ${days} days`;
  return `${formatDateTime(expiresAt)}, ${remaining}`;
}

/**
 * Whether an active issuer stops being active soon, unless it is renewed: from its
 * `renew_from`, which the backend sets from the sandbox lifetime, until its expiry.
 */
export function expiresSoon(
  issuer: Pick<Issuer, 'state' | 'expires_at' | 'renew_from'>,
  now = new Date(),
): boolean {
  if (issuer.state !== 'active' || !issuer.expires_at || !issuer.renew_from) {
    return false;
  }
  const time = now.getTime();
  return (
    time >= new Date(issuer.renew_from).getTime() && time < new Date(issuer.expires_at).getTime()
  );
}

/** How much of a limit is used: `3 of 10`, or the count alone when there is no limit. */
export function usageText(used: number, limit: number | null): string {
  return limit === null ? `${used}` : `${used} of ${limit}`;
}
