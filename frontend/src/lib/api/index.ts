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

const FALLBACK_MESSAGE = 'Something went wrong. Please try again.';

/** What an error response tells the user. */
export function errorMessage(error: unknown): string {
  const detail = (error as Partial<Schemas['ApiErrorResponse']> | undefined)?.errors?.[0]?.detail;
  return detail ?? FALLBACK_MESSAGE;
}
