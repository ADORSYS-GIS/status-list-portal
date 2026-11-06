export type NoticeKind = 'info' | 'success';

interface Notice {
  kind: NoticeKind;
  message: string;
  /** The page that shows it. Leaving that page drops it. */
  path: string;
}

class Notices {
  current = $state<Notice | null>(null);
}

/** What the user's last action led to, shown on the page it led to. */
export const notices = new Notices();

export function flash(path: string, kind: NoticeKind, message: string) {
  notices.current = { kind, message, path };
}
