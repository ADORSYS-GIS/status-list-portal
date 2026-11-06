import createClient from 'openapi-fetch';

import type { components, paths } from './schema';

type Schemas = components['schemas'];

export type User = Schemas['MeResponse'];
export type Terms = Schemas['TermsResponse'];
export type Issuer = Schemas['IssuerResponse'];
export type IssuerState = Schemas['IssuerState'];
export type Registration = Schemas['RegistrationResponse'];
export type RegistrationState = Schemas['RegistrationState'];
export type SigningAlgorithm = Schemas['SigningAlgorithm'];
export type NewIssuerRequest = Schemas['NewIssuerRequest'];

export const SIGNING_ALGORITHMS: readonly SigningAlgorithm[] = ['ES256', 'ES384', 'EdDSA', 'PS256'];

/** A client of the portal's backend, which shares the page's origin. */
export function createApi(fetch: typeof globalThis.fetch = globalThis.fetch) {
  return createClient<paths>({ fetch });
}

export type Api = ReturnType<typeof createApi>;

/** Most pages of issuers to read: far more than a sandbox account fills. */
const MAX_ISSUER_PAGES = 20;

/** Every issuer of the signed-in user, deleted ones included, or the first page that failed. */
export async function allIssuers(
  api: Api,
): Promise<{ issuers: Issuer[] } | { status: number; error: unknown }> {
  let issuers: Issuer[] = [];
  let cursor: string | undefined;
  for (let page = 0; page < MAX_ISSUER_PAGES; page++) {
    const { data, error, response } = await api.GET('/api/v1/issuers', {
      params: { query: { cursor } },
    });
    if (error !== undefined) {
      return { status: response.status, error };
    }
    issuers = [...issuers, ...data.issuers];
    if (data.next_cursor === null) {
      return { issuers };
    }
    cursor = data.next_cursor;
  }
  throw new Error(`The list of issuers did not end after ${MAX_ISSUER_PAGES} pages.`);
}

const FALLBACK_MESSAGE = 'Something went wrong. Please try again.';

/** What an error response tells the user. */
export function errorMessage(error: unknown): string {
  const detail = (error as Partial<Schemas['ApiErrorResponse']> | undefined)?.errors?.[0]?.detail;
  return detail ?? FALLBACK_MESSAGE;
}
