<script lang="ts">
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
</script>

<svelte:head>
  <title>{page.status === 404 ? 'Not found' : 'Error'} · Status List Portal</title>
</svelte:head>

{#if page.status === 401}
  <h1>Please sign in again</h1>
  <p class="lead">{page.error?.message}</p>
  <p>
    <!-- The backend serves this URL: it starts the sign-in, so the browser leaves the app. -->
    <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
    <a class="button primary" href="/api/v1/session/github" data-sveltekit-reload>
      Sign in with GitHub
    </a>
  </p>
{:else if page.status === 404}
  <h1>This page does not exist</h1>
  <p class="lead">{page.error?.message ?? 'Check the address, or go back to your issuers.'}</p>
{:else}
  <h1>Something went wrong</h1>
  <p class="lead">{page.error?.message ?? 'Please try again in a moment.'}</p>
{/if}
<p><a href={resolve('/')}>Go to the home page</a></p>
