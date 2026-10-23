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
