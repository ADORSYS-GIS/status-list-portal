<script lang="ts">
  import { tick } from 'svelte';

  import { makeKeyPair, type KeyPair } from '#lib/key-pair.ts';
  import type { SigningAlgorithm } from '#lib/api/index.ts';

  import Icon from './Icon.svelte';
  import Notice from './Notice.svelte';

  /**
   * Gets the public key once the private key is downloaded: a key registered before its
   * private half was downloaded could never sign a token. Browsers cannot tell
   * whether the file was then saved.
   */
  let { alg = 'ES256', onkey }: { alg?: SigningAlgorithm; onkey: (publicJwk: string) => void } =
    $props();

  /** Some browsers read a download's URL after the click returns; FileSaver.js waits as long. */
  const REVOKE_DELAY_MS = 40_000;

  let pair = $state<KeyPair | null>(null);
  let making = $state(false);
  let failed = $state(false);
  let downloaded = $state(false);
  let firstDownload = $state<HTMLButtonElement>();
  const id = $props.id();
  const warning = `${id}-warning`;

  async function make() {
    // A second pair would replace the first after its private key may have been downloaded.
    making = true;
    try {
      pair = await makeKeyPair(alg);
    } catch {
      failed = true;
      return;
    } finally {
      making = false;
    }
    // The button that had focus is gone; the next step is downloading the key.
    await tick();
    firstDownload?.focus();
  }

  function download(name: string, text: string, type: string) {
    const url = URL.createObjectURL(new Blob([text], { type }));
    const link = document.createElement('a');
    link.href = url;
    link.download = name;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), REVOKE_DELAY_MS);
    if (pair && !downloaded) {
      downloaded = true;
      onkey(pair.publicJwk);
    }
  }
</script>

<div class="generator">
  {#if pair}
    {@const { privatePem, privateJwk } = pair}
    <!-- Read as the downloads' description: a live region added with its text is often not
         announced, and focus goes straight to the first download. -->
    <Notice kind="warning" announce={false}>
      <p id={warning}>
        <strong>Your private key exists only in the files you download here.</strong> The portal never
        receives it and cannot recover it. Keep it safe: it signs your issuer's tokens. Move the files
        out of Downloads, which often syncs to the cloud, to a folder only you can read.
      </p>
    </Notice>
    <!-- The JSON first: the token script on the issuer page reads it; the PEM is for other tools. -->
    <div class="downloads">
      <button
        bind:this={firstDownload}
        type="button"
        class="button"
        aria-describedby={warning}
        onclick={() => download('private-key.json', privateJwk, 'application/json')}
      >
        <Icon name="arrow-down-tray" /> Download private-key.json
      </button>
      <button
        type="button"
        class="button"
        aria-describedby={warning}
        onclick={() => download('private-key.pem', privatePem, 'application/x-pem-file')}
      >
        <Icon name="arrow-down-tray" /> Download private-key.pem
      </button>
    </div>
    <p class="hint" role="status">
      {downloaded
        ? 'The public key is in the form.'
        : 'The public key goes into the form once you download the private key.'}
    </p>
  {:else}
    <button type="button" class="button" onclick={make} disabled={making}>
      <Icon name="key" /> Make a key pair in this browser
    </button>
    {#if failed}
      <p class="field-error" role="alert">
        This browser could not make a key pair. Use the Node.js script below instead.
      </p>
    {/if}
  {/if}
</div>

<style>
  .generator {
    margin-bottom: var(--space-4);
  }

  .downloads {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
</style>
