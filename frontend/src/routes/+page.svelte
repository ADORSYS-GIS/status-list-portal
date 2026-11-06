<script lang="ts">
  import GitHubMark from '#lib/components/GitHubMark.svelte';
  import Notice from '#lib/components/Notice.svelte';
  import { SIGN_IN_ERRORS } from '#lib/sign-in.ts';

  let { data } = $props();
</script>

<svelte:head>
  <title>Status List Portal</title>
</svelte:head>

{#if data.signInError}
  <Notice kind="error">
    <p>
      {SIGN_IN_ERRORS[data.signInError]}
      {#if data.signInError === 'no_verified_email'}
        <a href="https://github.com/settings/emails">Open your GitHub email settings</a>
      {/if}
    </p>
  </Notice>
{/if}

<section class="hero">
  <h1>Try the Status List Server with a sandbox issuer</h1>
  <p class="lead">
    Register the public key you sign with, and get an issuer that can publish status lists on the
    sandbox. It takes a few minutes and costs nothing.
  </p>
  <!-- The backend serves this URL: it starts the sign-in, so the browser leaves the app. -->
  <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
  <a class="button primary" href="/api/v1/session/github" data-sveltekit-reload>
    <GitHubMark />
    Sign in with GitHub
  </a>
</section>

<section aria-labelledby="how">
  <h2 id="how">How it works</h2>
  <ol class="steps">
    <li>
      <h3>Sign in with GitHub</h3>
      <p>The portal reads your login and your verified email address, nothing else.</p>
    </li>
    <li>
      <h3>Register a public key</h3>
      <p>
        Accept the terms of use and paste the public key of a key pair you hold, as a JSON Web Key.
        The portal shows you how to make one if you need to.
      </p>
    </li>
    <li>
      <h3>Publish a status list</h3>
      <p>
        Your issuer gets an id and the details for its tokens. Sign a token with your private key,
        which never leaves your computer, and publish your first list.
      </p>
    </li>
  </ol>
</section>

<section aria-labelledby="limits">
  <h2 id="limits">What the sandbox allows</h2>
  <p class="muted">
    Sandbox issuers are for trying things out. Each account can have a few active issuers at a time,
    and each issuer stays active for 30 days unless you renew it.
  </p>
</section>

<style>
  .hero {
    padding: var(--space-8) 0 var(--space-12);
  }

  .hero h1 {
    font-size: clamp(1.875rem, 4vw, 2.5rem);
    max-width: 40rem;
  }

  section + section {
    margin-top: var(--space-8);
  }

  .steps {
    display: grid;
    gap: var(--space-4);
    grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
    margin: 0;
    padding: 0;
    list-style: none;
    counter-reset: step;
  }

  .steps li {
    padding: var(--space-6);
    border: 1px solid var(--color-border);
    border-radius: var(--radius);
    counter-increment: step;
  }

  .steps h3::before {
    content: counter(step) '. ';
    color: var(--color-link);
  }

  .steps p {
    margin: 0;
    color: var(--color-muted);
  }
</style>
