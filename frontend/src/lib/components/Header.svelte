<script lang="ts">
  import { resolve } from '$app/paths';
  import type { User } from '#lib/api/index.ts';

  import AdorsysLogo from './AdorsysLogo.svelte';
  import GitHubMark from './GitHubMark.svelte';
  import Icon from './Icon.svelte';

  let { user, onsignout }: { user: User | null; onsignout: () => void } = $props();
</script>

<header>
  <div class="bar">
    <a class="brand" href={resolve('/')}>
      <AdorsysLogo />
      <span class="product">Status List Portal</span>
    </a>
    <nav aria-label="Account">
      {#if user}
        <a href={resolve('/issuers')}>Issuers</a>
        <span class="login">Signed in as <strong>{user.login}</strong></span>
        <button type="button" class="button" onclick={onsignout}>
          <Icon name="arrow-right-start-on-rectangle" />
          Sign out
        </button>
      {:else}
        <!-- The backend serves this URL: it starts the sign-in, so the browser leaves the app. -->
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
        <a class="button primary" href="/api/v1/session/github" data-sveltekit-reload>
          <GitHubMark />
          Sign in with GitHub
        </a>
      {/if}
    </nav>
  </div>
</header>

<style>
  header {
    border-bottom: 1px solid var(--color-border);
    background: var(--color-bg);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    max-width: var(--content-width);
    margin: 0 auto;
    padding: var(--space-3) var(--space-4);
  }

  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    color: var(--color-text);
    font-weight: 600;
    font-size: 1.0625rem;
    text-decoration: none;
  }

  .product {
    padding-left: var(--space-3);
    border-left: 1px solid var(--color-border);
  }

  nav {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-4);
  }

  .login {
    color: var(--color-muted);
    font-size: 0.9375rem;
  }

  @media (max-width: 40rem) {
    .login {
      display: none;
    }
  }
</style>
