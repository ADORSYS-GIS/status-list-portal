<script lang="ts">
  import type { IssuerState, RegistrationState } from '#lib/api/index.ts';

  let { state }: { state: IssuerState | RegistrationState } = $props();

  /** The words for each state, so that color is never the only signal. */
  const LABELS: Record<IssuerState | RegistrationState, string> = {
    active: 'Active',
    expired: 'Expired',
    suspended: 'Deleted',
    pending: 'Waiting for approval',
    approved: 'Approved',
    rejected: 'Rejected',
  };
  const TONES: Record<IssuerState | RegistrationState, string> = {
    active: 'success',
    approved: 'success',
    expired: 'warning',
    pending: 'warning',
    suspended: 'neutral',
    rejected: 'danger',
  };
</script>

<span class="badge {TONES[state]}">{LABELS[state]}</span>

<style>
  .badge {
    display: inline-block;
    padding: 0 var(--space-2);
    border: 1px solid currentColor;
    border-radius: 999px;
    font-size: 0.8125rem;
    font-weight: 500;
    white-space: nowrap;
  }

  .success {
    color: var(--color-success);
    background: var(--color-success-bg);
  }

  .warning {
    color: var(--color-warning);
    background: var(--color-warning-bg);
  }

  .danger {
    color: var(--color-danger);
    background: var(--color-danger-bg);
  }

  .neutral {
    color: var(--color-muted);
    background: var(--color-surface);
  }
</style>
