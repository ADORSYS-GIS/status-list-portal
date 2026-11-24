<script lang="ts">
  import { resolve } from '$app/paths';
  import { goto, invalidateAll } from '$app/navigation';
  import { page } from '$app/state';
  import {
    createApi,
    errorMessage,
    SIGNING_ALGORITHMS,
    type SigningAlgorithm,
  } from '#lib/api/index.ts';
  import CodeBlock from '#lib/components/CodeBlock.svelte';
  import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
  import Icon from '#lib/components/Icon.svelte';
  import IssuerFacts from '#lib/components/IssuerFacts.svelte';
  import Notice from '#lib/components/Notice.svelte';
  import StateBadge from '#lib/components/StateBadge.svelte';
  import {
    browserKeyPair,
    publicKeyProblem,
    publishCommands,
    signingKey,
    tokenScript,
  } from '#lib/keys.ts';
  import { flash } from '#lib/notices.svelte.ts';

  let { data } = $props();

  let issuer = $derived(data.issuer);
  let name = $derived(issuer.display_name ?? issuer.id);
  let busy = $state(false);
  let failure = $state<string | null>(null);
  let confirmDelete: ConfirmDialog;
  let confirmRevoke: ConfirmDialog;
  let revokeKid = $state('');
  let selectedKid = $state('');
  let keyFileName = $state('private-key.json');
  let generatedFiles = $state<Record<string, string>>({});
  let generatedFileName = $state('');
  let alg = $state<SigningAlgorithm>('ES256');
  let publicKey = $state('');
  let generated = $state(false);
  let keyError = $state<string | null>(null);
  let selectedKey = $derived(signingKey(issuer, selectedKid) ?? signingKey(issuer));
  $effect(() => {
    if (!selectedKid && selectedKey) selectedKid = selectedKey.kid;
  });
  let atKeyLimit = $derived(
    issuer.usage.keys_limit !== null && issuer.usage.keys >= issuer.usage.keys_limit,
  );

  let path = $derived({ params: { path: { id: issuer.id } } });

  async function renew() {
    await act(async () => {
      const { error } = await createApi().POST('/api/v1/issuers/{id}/renew', path);
      if (error) {
        failure = errorMessage(error);
        return;
      }
      flash(page.url.pathname, 'success', 'Renewed. The new expiry date is below.');
      await invalidateAll();
    });
  }

  async function remove() {
    await act(async () => {
      const { error } = await createApi().DELETE('/api/v1/issuers/{id}', path);
      if (error) {
        failure = errorMessage(error);
        return;
      }
      flash('/issuers', 'success', `${name} is deleted. Its tokens are no longer accepted.`);
      await goto(resolve('/issuers'), { invalidateAll: true });
    });
  }

  async function generate() {
    keyError = null;
    try {
      const pair = await browserKeyPair(alg);
      const filename = `private-key-${crypto.randomUUID()}.json`;
      const blob = new Blob([JSON.stringify(pair.privateKey)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = filename;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
      publicKey = JSON.stringify(pair.publicKey, null, 2);
      generatedFileName = filename;
      generated = true;
    } catch {
      keyError = 'Your browser could not make this key pair. Paste a public key instead.';
    }
  }

  async function addKey(event: SubmitEvent) {
    event.preventDefault();
    keyError = publicKeyProblem(publicKey);
    if (keyError || atKeyLimit) return;
    await act(async () => {
      const { data: added, error } = await createApi().POST('/api/v1/issuers/{id}/keys', {
        ...path,
        body: { alg, public_key: JSON.parse(publicKey) },
      });
      if (error) {
        failure = errorMessage(error);
        return;
      }
      if (generated && added) generatedFiles[added.kid] = generatedFileName;
      if (!selectedKey && added) selectKey(added.kid);
      publicKey = '';
      generated = false;
      flash(
        page.url.pathname,
        'success',
        'Key added. Move your signers to it and verify a write before revoking the old key.',
      );
      await invalidateAll();
    });
  }

  function selectKey(kid: string) {
    selectedKid = kid;
    keyFileName = generatedFiles[kid] ?? 'private-key.json';
  }

  function askToRevoke(kid: string) {
    revokeKid = kid;
    const active = issuer.keys.filter((key) => key.state === 'active');
    if (active.length === 1 && active[0].kid === kid) confirmRevoke.open();
    else void revokeKey();
  }

  async function revokeKey() {
    const kid = revokeKid;
    await act(async () => {
      const { error } = await createApi().POST('/api/v1/issuers/{id}/keys/{kid}/revoke', {
        params: { path: { id: issuer.id, kid } },
      });
      if (error) {
        failure = errorMessage(error);
        return;
      }
      if (selectedKid === kid) {
        const other = issuer.keys.find((key) => key.state === 'active' && key.kid !== kid);
        if (other) selectKey(other.kid);
        else selectedKid = '';
      }
      flash(
        page.url.pathname,
        'success',
        'Key revoked. Tokens signed with it are no longer accepted.',
      );
      await invalidateAll();
    });
  }

  /** Runs `action` with the buttons disabled. A failed request is reported, never lost. */
  async function act(action: () => Promise<void>) {
    busy = true;
    failure = null;
    try {
      await action();
    } catch {
      failure = errorMessage(undefined);
    } finally {
      busy = false;
    }
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

<IssuerFacts {issuer} selectedKid={selectedKey?.kid} />

{#if issuer.state !== 'suspended'}
  <section aria-labelledby="keys">
    <h2 id="keys">Signing keys</h2>
    <p>
      {issuer.usage.keys} of {issuer.usage.keys_limit ?? 'unlimited'} key slots used. Revoked keys do
      not use a slot.
    </p>
    <ol class="key-list">
      {#each issuer.keys as key (key.kid)}
        <li>
          <span class="mono">{key.kid}</span> · {key.alg} · {key.state}
          {#if key.state === 'active'}
            <button type="button" onclick={() => selectKey(key.kid)} disabled={busy}
              >Use in example</button
            >
          {/if}
          {#if key.state !== 'revoked'}
            <button type="button" onclick={() => askToRevoke(key.kid)} disabled={busy}
              >Revoke</button
            >
          {/if}
        </li>
      {/each}
    </ol>
    {#if atKeyLimit}
      <Notice kind="info"
        ><p>
          The key limit is reached. Keep a working key active while freeing a slot; rotation without
          a spare slot would interrupt writes.
        </p></Notice
      >
    {:else}
      <form onsubmit={addKey}>
        <h3>Add a key</h3>
        <label for="key-alg">Algorithm</label>
        <select id="key-alg" bind:value={alg}>
          {#each SIGNING_ALGORITHMS as algorithm (algorithm)}<option value={algorithm}
              >{algorithm}</option
            >{/each}
        </select>
        <button type="button" onclick={generate} disabled={busy}
          >Make a key pair in this browser</button
        >
        <label for="new-public-key">Public key (JWK)</label>
        <textarea
          id="new-public-key"
          bind:value={publicKey}
          oninput={() => {
            generated = false;
            keyError = null;
          }}
          aria-invalid={keyError ? 'true' : undefined}></textarea>
        {#if keyError}<p class="field-error">{keyError}</p>{/if}
        {#if generated}
          <p>
            Save the downloaded <code>{generatedFileName}</code> before adding this key. Its private half
            stays in your browser and the downloaded file.
          </p>
        {/if}
        <button type="submit" class="button" disabled={busy || !publicKey.trim()}
          >Add public key</button
        >
      </form>
    {/if}
    <h3>Rotate without interrupting writes</h3>
    <ol>
      <li>Add the new public key while the old key remains active.</li>
      <li>Put the new private key on every signer and select its key id for tokens.</li>
      <li>Verify a status list write with the new key on the sandbox.</li>
      <li>After all signers have switched, revoke the old key.</li>
    </ol>
  </section>

  {#if selectedKey}
    <section aria-labelledby="publish">
      <h2 id="publish">Publish your first status list</h2>
      <p>
        In the folder with your <code>private-key.json</code>, save this script as
        <code>mint-token.mjs</code>. It signs a management token that lasts two minutes.
      </p>
      <p>
        Example key: <code>{selectedKey.kid}</code>. Select another active key above if your signer
        has not switched yet.
      </p>
      <label for="private-key-file">Private key filename for this example</label>
      <input id="private-key-file" bind:value={keyFileName} />
      <CodeBlock code={tokenScript(issuer, selectedKey.kid, keyFileName)} title="mint-token.mjs" />
      <p>Then publish a status list with one valid entry:</p>
      <CodeBlock code={publishCommands(issuer)} title="Publish a status list" />
    </section>
  {:else}
    <Notice kind="info"
      ><p>No active signing key is available. Add one before writing status lists.</p></Notice
    >
  {/if}

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

<ConfirmDialog
  bind:this={confirmRevoke}
  title="Revoke the last active key?"
  confirmLabel="Revoke key"
  onconfirm={revokeKey}
>
  <p>This issuer will no longer be able to write status lists until you add a new key.</p>
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
  .key-list li {
    margin-bottom: var(--space-3);
    overflow-wrap: anywhere;
  }
  .key-list button {
    margin-left: var(--space-3);
  }
  form {
    display: grid;
    gap: var(--space-3);
    max-width: 36rem;
  }
  textarea {
    min-height: 8rem;
  }
</style>
