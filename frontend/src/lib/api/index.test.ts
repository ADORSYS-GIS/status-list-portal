import createClient from 'openapi-fetch';
import { expect, test } from 'vitest';

import { allIssuers, type Issuer } from './index';
import type { paths } from './schema';

function issuer(id: string): Issuer {
  return {
    id,
    display_name: id,
    state: 'active',
    created_at: '2026-11-01T00:00:00Z',
    expires_at: '2026-12-01T00:00:00Z',
    keys: [],
    management_url: 'https://status.example.com',
    audience: 'https://status.example.com',
  };
}

/** A client whose backend answers with `pages`, keyed by the cursor they are read with. */
function apiWith(pages: Record<string, { status?: number; body: unknown }>) {
  const read: string[] = [];
  const fetch = async (request: Request) => {
    const cursor = new URL(request.url).searchParams.get('cursor') ?? '';
    read.push(cursor);
    const page = pages[cursor];
    return Response.json(page.body, { status: page.status ?? 200 });
  };
  return { api: createClient<paths>({ baseUrl: 'http://portal.test', fetch }), read };
}

test('every page of issuers is read', async () => {
  const { api, read } = apiWith({
    '': { body: { issuers: [issuer('a'), issuer('b')], next_cursor: 'second' } },
    second: { body: { issuers: [], next_cursor: 'third' } },
    third: { body: { issuers: [issuer('c')], next_cursor: null } },
  });

  const result = await allIssuers(api);

  expect(result).toEqual({ issuers: [issuer('a'), issuer('b'), issuer('c')] });
  expect(read).toEqual(['', 'second', 'third']);
});

test('a page that fails stops the reading', async () => {
  const failure = { errors: [{ code: 'unauthorized', detail: 'Not signed in.' }] };
  const { api } = apiWith({
    '': { body: { issuers: [issuer('a')], next_cursor: 'second' } },
    second: { status: 401, body: failure },
  });

  const result = await allIssuers(api);

  expect(result).toEqual({ status: 401, error: failure });
});

test('a list that never ends is not read forever', async () => {
  const { api } = apiWith({
    '': { body: { issuers: [issuer('a')], next_cursor: 'again' } },
    again: { body: { issuers: [issuer('b')], next_cursor: 'again' } },
  });

  await expect(allIssuers(api)).rejects.toThrow(/did not end/);
});
