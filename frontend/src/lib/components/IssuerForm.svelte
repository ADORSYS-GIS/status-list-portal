<script lang="ts" module>
  export interface IssuerFormValues {
    displayName: string;
    alg: SigningAlgorithm;
    publicKey: string;
  }
</script>

<script lang="ts">
  import { tick } from 'svelte';

  import { SIGNING_ALGORITHMS, type SigningAlgorithm, type Terms } from '#lib/api/index.ts';
  import { publicKeyProblem } from '#lib/keys.ts';

  import KeyPairGenerator from './KeyPairGenerator.svelte';
  import KeyPairHelp from './KeyPairHelp.svelte';

  const DISPLAY_NAME_MAX = 100;
  const ALGORITHM_LABELS: Record<SigningAlgorithm, string> = {
    ES256: 'ES256: ECDSA on P-256, a good default',
    ES384: 'ES384: ECDSA on P-384',
    EdDSA: 'EdDSA: Ed25519',
    PS256: 'PS256: RSA-PSS, 2048 bits or more',
  };

  let {
    terms,
    termsAccepted,
    submitting,
    onsubmit,
  }: {
    terms: Terms;
    termsAccepted: boolean;
    submitting: boolean;
    onsubmit: (values: IssuerFormValues) => void;
  } = $props();

  let displayName = $state('');
  let alg = $state<SigningAlgorithm>('ES256');
  let publicKey = $state('');
  let acceptTerms = $state(false);
  // A key made here stays as made: its saved private half signs for nothing else.
  let generated = $state(false);
  // Changing it remounts the generator, which drops the key pair it holds.
  let generator = $state(0);
  // Shown once a submit fails, then kept up to date as the fields change.
  let submitted = $state(false);
  const errors = $derived(submitted ? check() : {});
  let form: HTMLFormElement;
  let publicKeyField: HTMLTextAreaElement;

  function check(): { displayName?: string; publicKey?: string; terms?: string } {
    const name = displayName.trim();
    return {
      displayName:
        name.length === 0 || name.length > DISPLAY_NAME_MAX
          ? `Give your issuer a name of 1 to ${DISPLAY_NAME_MAX} characters.`
          : undefined,
      publicKey: publicKeyProblem(publicKey) ?? undefined,
      terms: termsAccepted || acceptTerms ? undefined : 'Accept the terms of use to continue.',
    };
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const problems = check();
    if (problems.displayName || problems.publicKey || problems.terms) {
      submitted = true;
      await tick();
      form.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
      return;
    }
    onsubmit({ displayName: displayName.trim(), alg, publicKey });
  }

  function useGeneratedKey(publicJwk: string) {
    publicKey = publicJwk;
    alg = 'ES256';
    generated = true;
  }

  async function useOwnKey() {
    publicKey = '';
    generated = false;
    generator += 1;
    // The button that had focus is gone; the next step is pasting a key.
    await tick();
    publicKeyField.focus();
  }
</script>

<form bind:this={form} onsubmit={submit} novalidate>
  <div class="field">
    <label for="display-name">Display name</label>
    <p class="hint" id="display-name-hint">Shown to you and to the sandbox operators.</p>
    <input
      id="display-name"
      bind:value={displayName}
      maxlength={DISPLAY_NAME_MAX}
      autocomplete="off"
      aria-invalid={errors.displayName ? 'true' : undefined}
      aria-describedby="display-name-hint{errors.displayName ? ' display-name-error' : ''}"
    />
    {#if errors.displayName}
      <p class="field-error" id="display-name-error">{errors.displayName}</p>
    {/if}
  </div>

  <div class="field">
    <label for="alg">Algorithm</label>
    <p class="hint" id="alg-hint">The algorithm your issuer signs its tokens with.</p>
    <select id="alg" bind:value={alg} disabled={generated} aria-describedby="alg-hint">
      {#each SIGNING_ALGORITHMS as option (option)}
        <option value={option}>{ALGORITHM_LABELS[option]}</option>
      {/each}
    </select>
  </div>

  <div class="field">
    <label for="public-key">Public key</label>
    <p class="hint" id="public-key-hint">
      The public key of your key pair, as a JSON Web Key. Never paste the private key.
    </p>
    <textarea
      id="public-key"
      bind:this={publicKeyField}
      bind:value={publicKey}
      rows="6"
      readonly={generated}
      spellcheck="false"
      placeholder={'{"kty":"EC","crv":"P-256","x":"…","y":"…"}'}
      aria-invalid={errors.publicKey ? 'true' : undefined}
      aria-describedby="public-key-hint{errors.publicKey ? ' public-key-error' : ''}"></textarea>
    {#if errors.publicKey}
      <p class="field-error" id="public-key-error">{errors.publicKey}</p>
    {/if}
    {#if generated}
      <button type="button" class="button own-key" onclick={useOwnKey}>
        Use your own key instead
      </button>
    {/if}
  </div>

  {#key generator}
    <KeyPairGenerator onkey={useGeneratedKey} />
  {/key}
  <KeyPairHelp {alg} />

  {#if !termsAccepted}
    <div class="field">
      <div class="checkbox">
        <input
          id="accept-terms"
          type="checkbox"
          bind:checked={acceptTerms}
          aria-invalid={errors.terms ? 'true' : undefined}
          aria-describedby={errors.terms ? 'terms-error' : undefined}
        />
        <label for="accept-terms">
          I have read and accept the <a href={terms.url} rel="external">terms of use</a>
          (version {terms.version}).
        </label>
      </div>
      {#if errors.terms}
        <p class="field-error" id="terms-error">{errors.terms}</p>
      {/if}
    </div>
  {/if}

  <button type="submit" class="button primary" disabled={submitting}>
    {submitting ? 'Registering…' : 'Register issuer'}
  </button>
</form>

<style>
  form {
    max-width: 40rem;
  }

  .own-key {
    justify-self: start;
  }

  .checkbox {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
  }

  .checkbox input {
    width: 1.25rem;
    min-height: 1.25rem;
    margin-top: 0.2rem;
    flex: none;
  }

  .checkbox label {
    font-weight: 400;
  }
</style>
