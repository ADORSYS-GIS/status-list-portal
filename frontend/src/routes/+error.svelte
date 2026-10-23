<script lang="ts">
  import { resolve } from '$app/paths';
  import { page } from '$app/state';
</script>

<svelte:head>
  <title>{page.status === 404 ? 'Not found' : 'Error'} · Status List Portal</title>
</svelte:head>

{#if page.status === 404}
  <h1>This page does not exist</h1>
  <p class="lead">{page.error?.message ?? 'Check the address, or go back to your issuers.'}</p>
{:else}
  <h1>Something went wrong</h1>
  <p class="lead">{page.error?.message ?? 'Please try again in a moment.'}</p>
{/if}
<p><a href={resolve('/')}>Go to the home page</a></p>
