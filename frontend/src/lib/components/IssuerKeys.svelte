<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { invalidateAll } from '$app/navigation';
  import { page } from '$app/state';
  import {
    createApi,
    errorMessage,
    SIGNING_ALGORITHMS,
    type Issuer,
    type SigningAlgorithm,
  } from '#lib/api/index.ts';
  import { ALGORITHM_LABELS, browserKeyPair, publicKeyProblem, signingKey } from '#lib/keys.ts';
  import { flash } from '#lib/notices.svelte.ts';

  import ConfirmDialog from './ConfirmDialog.svelte';
  import Notice from './Notice.svelte';

  let {
    issuer,
    issuerVersion,
    selectedKid = $bindable(''),
    keyFileName = $bindable('private-key.json'),
  }: {
    issuer: Issuer;
    issuerVersion: string;
    selectedKid?: string;
    keyFileName?: string;
  } = $props();

  let busy = $state(false);
  let failure = $state<string | null>(null);
  let confirmRevoke: ConfirmDialog;
  let revokeKid = $state('');
  let generatedFiles = $state<Record<string, string>>({});
  let generatedFileName = $state('');
  let alg = $state<SigningAlgorithm>(untrack(() => signingKey(issuer)?.alg ?? 'ES256'));
  let publicKey = $state('');
  let generated = $state(false);
  let keyError = $state<string | null>(null);
  let publicKeyField = $state<HTMLTextAreaElement>();
  let lastActiveRevoke = $derived(issuer.keys.filter((key) => key.state === 'active').length === 1);
  let atKeyLimit = $derived(
    issuer.usage.keys_limit !== null && issuer.usage.keys >= issuer.usage.keys_limit,
  );

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
    if (keyError) {
      await tick();
      publicKeyField?.focus();
      return;
    }
    if (atKeyLimit) return;
    await act(async () => {
      const { data: added, error } = await createApi().POST('/api/v1/issuers/{id}/keys', {
        params: { path: { id: issuer.id }, header: { 'if-issuer-version': issuerVersion } },
        body: { alg, public_key: JSON.parse(publicKey) },
      });
      if (error) {
        failure = errorMessage(error);
        return;
      }
      if (generated && added) generatedFiles[added.kid] = generatedFileName;
      if (!signingKey(issuer) && added) selectKey(added.kid);
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
    confirmRevoke.open();
  }

  async function revokeKey() {
    const kid = revokeKid;
    await act(async () => {
      const { error } = await createApi().POST('/api/v1/issuers/{id}/keys/{kid}/revoke', {
        params: { path: { id: issuer.id, kid }, header: { 'if-issuer-version': issuerVersion } },
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

<section aria-labelledby="keys">
  <h2 id="keys">Signing keys</h2>
  {#if failure}<Notice kind="error"><p>{failure}</p></Notice>{/if}
  <p>Revoked keys do not use a slot.</p>
  <ol class="key-list">
    {#each issuer.keys as key (key.kid)}
      <li>
        <span class="mono">{key.kid}</span> · {key.alg} · {key.state}
        {#if key.state === 'active'}
          <button
            type="button"
            class="button"
            aria-label="Use in example: key {key.kid}"
            onclick={() => selectKey(key.kid)}
            disabled={busy}>Use in example</button
          >
          <button
            type="button"
            class="button"
            aria-label="Revoke key {key.kid}"
            onclick={() => askToRevoke(key.kid)}
            disabled={busy}>Revoke</button
          >
        {/if}
      </li>
    {/each}
  </ol>
  {#if atKeyLimit}
    <Notice kind="info"
      ><p>
        The key limit is reached. Keep a working key active while freeing a slot; rotation without a
        spare slot would interrupt writes.
      </p></Notice
    >
  {:else}
    <form onsubmit={addKey} novalidate>
      <h3>Add a key</h3>
      <div class="field">
        <label for="key-alg">Algorithm</label>
        <p class="hint" id="key-alg-hint">The algorithm the new key signs tokens with.</p>
        <select
          id="key-alg"
          bind:value={alg}
          aria-describedby="key-alg-hint"
          onchange={() => {
            if (generated) {
              publicKey = '';
              generated = false;
              generatedFileName = '';
            }
            keyError = null;
          }}
        >
          {#each SIGNING_ALGORITHMS as algorithm (algorithm)}
            <option value={algorithm}>{ALGORITHM_LABELS[algorithm]}</option>
          {/each}
        </select>
      </div>
      <p class="hint" id="generate-key-hint">
        Save the downloaded private key. It is the only copy; the browser cannot restore it after
        this page closes.
      </p>
      <button
        type="button"
        class="button"
        onclick={generate}
        disabled={busy}
        aria-describedby="generate-key-hint">Make a key pair in this browser</button
      >
      <div class="field">
        <label for="new-public-key">Public key (JWK)</label>
        <p class="hint" id="new-public-key-hint">Paste only the public key as a JSON Web Key.</p>
        <textarea
          id="new-public-key"
          bind:this={publicKeyField}
          bind:value={publicKey}
          rows="6"
          spellcheck="false"
          oninput={() => {
            generated = false;
            keyError = null;
          }}
          aria-invalid={keyError ? 'true' : undefined}
          aria-describedby="new-public-key-hint{keyError ? ' new-public-key-error' : ''}"
        ></textarea>
        {#if keyError}<p class="field-error" id="new-public-key-error">{keyError}</p>{/if}
      </div>
      {#if generated}
        <p>
          Keep the downloaded <code>{generatedFileName}</code> safe. It holds the only private half.
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

<ConfirmDialog
  bind:this={confirmRevoke}
  title="Revoke key {revokeKid}?"
  confirmLabel="Revoke key"
  onconfirm={revokeKey}
>
  {#if lastActiveRevoke}
    <p>This issuer will no longer be able to write status lists until you add a new key.</p>
  {:else}
    <p>
      Tokens signed with this key will stop working immediately. Switch every signer before revoking
      it.
    </p>
  {/if}
</ConfirmDialog>

<style>
  section {
    margin-top: var(--space-8);
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
</style>
