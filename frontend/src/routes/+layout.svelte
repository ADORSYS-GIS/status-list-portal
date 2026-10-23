<script lang="ts">
  import { resolve } from '$app/paths';
  import '@fontsource-variable/ibm-plex-sans';
  import '@fontsource-variable/jetbrains-mono';
  import '#lib/css/global.css';

  import { afterNavigate, goto } from '$app/navigation';
  import { page } from '$app/state';
  import { createApi } from '#lib/api/index.ts';
  import Footer from '#lib/components/Footer.svelte';
  import Header from '#lib/components/Header.svelte';
  import Notice from '#lib/components/Notice.svelte';
  import { notices } from '#lib/notices.svelte.ts';

  let { data, children } = $props();

  let main: HTMLElement;

  afterNavigate(({ from, to }) => {
    if (notices.current && notices.current.path !== to?.url.pathname) {
      notices.current = null;
    }
    // Screen readers start reading the new page, rather than stay where the link was.
    if (from) {
      main.focus();
    }
  });

  async function signOut() {
    await createApi().DELETE('/api/v1/session');
    await goto(resolve('/'), { invalidateAll: true });
  }
</script>

<a class="skip" href="#main">Skip to content</a>
<Header user={data.user} onsignout={signOut} />
<main id="main" class="page" tabindex="-1" bind:this={main}>
  {#if notices.current && notices.current.path === page.url.pathname}
    <Notice kind={notices.current.kind}><p>{notices.current.message}</p></Notice>
  {/if}
  {@render children()}
</main>
<Footer terms={data.terms} />

<style>
  .skip {
    position: absolute;
    left: var(--space-4);
    top: -3rem;
    padding: var(--space-2) var(--space-4);
    background: var(--color-accent);
    color: var(--color-on-accent);
    border-radius: var(--radius);
    z-index: 10;
  }

  .skip:focus {
    top: var(--space-2);
  }

  main:focus {
    outline: none;
  }
</style>
