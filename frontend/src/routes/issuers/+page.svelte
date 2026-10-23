<script lang="ts">
  import { resolve } from '$app/paths';
  import { invalidateAll } from '$app/navigation';
  import { createApi, errorMessage, type Registration } from '#lib/api/index.ts';
  import Icon from '#lib/components/Icon.svelte';
  import IssuerTable from '#lib/components/IssuerTable.svelte';
  import Notice from '#lib/components/Notice.svelte';
  import RegistrationList from '#lib/components/RegistrationList.svelte';

  let { data } = $props();

  let failure = $state<string | null>(null);

  let waiting = $derived(data.registrations.filter((registration) => !registration.issuer_id));

  async function withdraw(registration: Registration) {
    const { error } = await createApi().DELETE('/api/v1/registrations/{id}', {
      params: { path: { id: registration.id } },
    });
    failure = error ? errorMessage(error) : null;
    await invalidateAll();
  }
</script>

<svelte:head>
  <title>Your issuers · Status List Portal</title>
</svelte:head>

<div class="heading">
  <h1>Your issuers</h1>
  <a class="button primary" href={resolve('/issuers/new')}>
    <Icon name="plus" />
    Register an issuer
  </a>
</div>

{#if failure}
  <Notice kind="error"><p>{failure}</p></Notice>
{/if}

{#if data.issuers.length > 0}
  <IssuerTable issuers={data.issuers} />
{:else}
  <div class="empty">
    <h2>No issuers yet</h2>
    <p class="muted">Register one to start publishing status lists on the sandbox.</p>
  </div>
{/if}

{#if waiting.length > 0}
  <section aria-labelledby="registrations">
    <h2 id="registrations">Registrations</h2>
    <p class="muted">
      An operator reviews each registration before its issuer is created. Rejected ones count toward
      the issuers your account may register.
    </p>
    <RegistrationList registrations={waiting} onwithdraw={withdraw} />
  </section>
{/if}

<style>
  .heading {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    margin-bottom: var(--space-6);
  }

  .heading h1 {
    margin: 0;
  }

  .empty {
    padding: var(--space-8);
    border: 1px dashed var(--color-border);
    border-radius: var(--radius);
    text-align: center;
  }

  .empty p {
    margin: 0;
  }

  section {
    margin-top: var(--space-12);
  }
</style>
