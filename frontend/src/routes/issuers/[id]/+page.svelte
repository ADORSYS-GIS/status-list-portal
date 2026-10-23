<script lang="ts">
  import { resolve } from '$app/paths';
  import { goto, invalidateAll } from '$app/navigation';
  import { page } from '$app/state';
  import { createApi, errorMessage } from '#lib/api/index.ts';
  import CodeBlock from '#lib/components/CodeBlock.svelte';
  import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import IssuerFacts from '#lib/components/IssuerFacts.svelte';
  import Notice from '#lib/components/Notice.svelte';
  import StateBadge from '#lib/components/StateBadge.svelte';
  import { publishCommands, tokenScript } from '#lib/keys.ts';
  import { flash } from '#lib/notices.svelte.ts';

  let { data } = $props();

  let issuer = $derived(data.issuer);
  let name = $derived(issuer.display_name ?? issuer.id);
  let busy = $state(false);
  let failure = $state<string | null>(null);
  let confirmDelete: ConfirmDialog;

  let path = $derived({ params: { path: { id: issuer.id } } });

  async function renew() {
    busy = true;
    const { error } = await createApi().POST('/api/v1/issuers/{id}/renew', path);
    busy = false;
    failure = error ? errorMessage(error) : null;
    if (!error) {
      flash(page.url.pathname, 'success', 'Renewed for 30 more days.');
      await invalidateAll();
    }
  }

  async function remove() {
    busy = true;
    const { error } = await createApi().DELETE('/api/v1/issuers/{id}', path);
    busy = false;
    if (error) {
      failure = errorMessage(error);
      return;
    }
    flash('/issuers', 'success', `${name} is deleted. Its tokens are no longer accepted.`);
    await goto(resolve('/issuers'), { invalidateAll: true });
  }
</script>

<svelte:head>
  <title>{name} · Status List Portal</title>
</svelte:head>

<p><a href={resolve('/issuers')}>← Your issuers</a></p>
<div class="heading">
  <h1>{name}</h1>
  <StateBadge state={issuer.state} />
</div>

{#if failure}
  <Notice kind="error"><p>{failure}</p></Notice>
{/if}

<IssuerFacts {issuer} />

{#if issuer.state !== 'suspended'}
  <section aria-labelledby="publish">
    <h2 id="publish">Publish your first status list</h2>
    <p>
      In the folder with your <code>private-key.json</code>, save this script as
      <code>mint-token.mjs</code>. It signs a management token that lasts two minutes.
    </p>
    <CodeBlock code={tokenScript(issuer)} title="mint-token.mjs" />
    <p>Then publish a status list with one valid entry:</p>
    <CodeBlock code={publishCommands(issuer)} title="Publish a status list" />
  </section>

  <section aria-labelledby="manage">
    <h2 id="manage">Manage</h2>
    <div class="actions">
      <button type="button" class="button" onclick={renew} disabled={busy}>
        <Icon name="arrow-path" />
        Renew for 30 days
      </button>
      <button type="button" class="button" onclick={() => confirmDelete.open()} disabled={busy}>
        <Icon name="trash" />
        Delete issuer
      </button>
    </div>
  </section>
{:else}
  <Notice kind="info"
    ><p>This issuer is deleted. The server no longer accepts its tokens.</p></Notice
  >
{/if}

<ConfirmDialog
  bind:this={confirmDelete}
  title="Delete {name}?"
  confirmLabel="Delete issuer"
  cancelLabel="Keep issuer"
  onconfirm={remove}
>
  <p>
    The server stops accepting its tokens at once. This cannot be undone, and its key cannot
    register another issuer: a new one needs a new key pair.
  </p>
</ConfirmDialog>

<style>
  .heading {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-6);
  }

  .heading h1 {
    margin: 0;
    overflow-wrap: anywhere;
  }

  section {
    margin-top: var(--space-8);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
</style>
