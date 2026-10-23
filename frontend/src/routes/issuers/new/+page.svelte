<script lang="ts">
  import { resolve } from '$app/paths';
  import { goto } from '$app/navigation';
  import { createApi, errorMessage, type Issuer } from '#lib/api/index.ts';
  import IssuerForm, { type IssuerFormValues } from '#lib/components/IssuerForm.svelte';
  import Notice from '#lib/components/Notice.svelte';
  import { flash } from '#lib/notices.svelte.ts';

  let { data } = $props();

  let submitting = $state(false);
  let failure = $state<string | null>(null);

  async function register(values: IssuerFormValues) {
    submitting = true;
    failure = null;
    try {
      const api = createApi();
      if (!data.user?.terms_accepted && data.terms) {
        const accepted = await api.PUT('/api/v1/me/terms', {
          body: { version: data.terms.version },
        });
        if (accepted.error) {
          failure = errorMessage(accepted.error);
          return;
        }
      }
      const {
        data: created,
        error,
        response,
      } = await api.POST('/api/v1/issuers', {
        body: {
          display_name: values.displayName,
          alg: values.alg,
          public_key: JSON.parse(values.publicKey),
        },
      });
      if (error) {
        failure = errorMessage(error);
        return;
      }
      if (response.status === 202) {
        flash(
          '/issuers',
          'info',
          'Thanks. An operator reviews your registration; its issuer appears here once approved.',
        );
        await goto(resolve('/issuers'), { invalidateAll: true });
        return;
      }
      const path = resolve('/issuers/[id]', { id: encodeURIComponent((created as Issuer).id) });
      flash(path, 'success', 'Your issuer is ready. Sign a token below to publish a status list.');
      await goto(path, { invalidateAll: true });
    } finally {
      submitting = false;
    }
  }
</script>

<svelte:head>
  <title>Register an issuer · Status List Portal</title>
</svelte:head>

<h1>Register an issuer</h1>
<p class="lead">
  Your issuer signs its management tokens with your private key. The portal only needs the public
  key.
</p>

{#if failure}
  <Notice kind="error"><p>{failure}</p></Notice>
{/if}

{#if !data.terms}
  <Notice kind="error"><p>The terms of use could not be loaded. Please reload the page.</p></Notice>
{:else if data.user}
  <IssuerForm
    terms={data.terms}
    termsAccepted={data.user.terms_accepted}
    {submitting}
    onsubmit={register}
  />
{/if}
