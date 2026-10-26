/**
 * Stand-ins for GitHub and the Status List Server's admin API, for the end-to-end tests:
 * `bun e2e/fakes.ts`.
 *
 * The stand-in for GitHub approves every sign-in at once, for the account that the
 * browser's `github_user` cookie names, as GitHub would for whoever is signed in. It checks
 * what a sign-in sends: the client credentials, the scope, and the PKCE verifier. The admin
 * API keeps issuers in memory, checks the portal's admin tokens, and refuses what the real
 * one would. `GET /e2e/issuers` shows the tests what it holds.
 */

import { createHash, createPublicKey, randomUUID, verify } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';

import { ADMIN_API, GITHUB, GITHUB_USER_COOKIE } from './settings.ts';

type Json = Record<string, unknown>;

interface Account {
  id: number;
  login: string;
  name: string;
  email: string;
}

async function readBody(request: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) {
    chunks.push(chunk as Buffer);
  }
  return Buffer.concat(chunks).toString('utf8');
}

/** The request's JSON object, or `undefined` if it sent none. */
async function readJson(request: IncomingMessage): Promise<Json | undefined> {
  try {
    const value: unknown = JSON.parse(await readBody(request));
    return typeof value === 'object' && value !== null && !Array.isArray(value)
      ? (value as Json)
      : undefined;
  } catch {
    return undefined;
  }
}

function send(
  response: ServerResponse,
  status: number,
  body?: unknown,
  headers: Record<string, string> = {},
) {
  const json = body === undefined ? '' : JSON.stringify(body);
  response.writeHead(status, {
    ...(json ? { 'content-type': 'application/json' } : {}),
    ...headers,
  });
  response.end(json);
}

function cookie(request: IncomingMessage, name: string): string | undefined {
  return request.headers.cookie
    ?.split(';')
    .map((pair) => pair.trim().split('='))
    .find(([key]) => key === name)?.[1];
}

function bearer(request: IncomingMessage): string | undefined {
  return request.headers.authorization?.match(/^Bearer (.+)$/i)?.[1];
}

// GitHub

const accounts = new Map<string, Account>();
const codes = new Map<string, { account: Account; challenge: string; redirectUri: string }>();
const tokens = new Map<string, Account>();
let nextAccountId = 1000;

function account(login: string): Account {
  const existing = accounts.get(login);
  if (existing) {
    return existing;
  }
  const created = {
    id: nextAccountId++,
    login,
    name: `Developer ${login}`,
    email: `${login}@example.com`,
  };
  accounts.set(login, created);
  return created;
}

/** Whether a sign-in asks for what the portal must: a code, the email scope, and PKCE. */
function validAuthorization(query: URLSearchParams): boolean {
  return (
    query.get('client_id') === GITHUB.clientId &&
    query.get('response_type') === 'code' &&
    query.get('scope') === 'user:email' &&
    query.get('code_challenge_method') === 'S256' &&
    Boolean(query.get('code_challenge')) &&
    Boolean(query.get('state')) &&
    Boolean(query.get('redirect_uri'))
  );
}

async function github(request: IncomingMessage, response: ServerResponse) {
  const url = new URL(request.url ?? '/', GITHUB.url);
  const route = `${request.method} ${url.pathname}`;

  if (route === 'GET /health') {
    return send(response, 200, {});
  }
  if (route === 'GET /login/oauth/authorize') {
    const query = url.searchParams;
    if (!validAuthorization(query)) {
      return send(response, 400, { error: 'invalid sign-in request' });
    }
    const login = cookie(request, GITHUB_USER_COOKIE) ?? `developer-${nextAccountId}`;
    const code = randomUUID();
    const redirectUri = query.get('redirect_uri') ?? '';
    codes.set(code, {
      account: account(login),
      challenge: query.get('code_challenge') ?? '',
      redirectUri,
    });
    const back = new URL(redirectUri);
    back.searchParams.set('code', code);
    back.searchParams.set('state', query.get('state') ?? '');
    response.writeHead(302, { location: back.href });
    response.end();
    return;
  }
  if (route === 'POST /login/oauth/access_token') {
    const form = new URLSearchParams(await readBody(request));
    const grant = codes.get(form.get('code') ?? '');
    const verifier = form.get('code_verifier') ?? '';
    const challenge = createHash('sha256').update(verifier).digest('base64url');
    const valid =
      grant &&
      form.get('client_id') === GITHUB.clientId &&
      form.get('client_secret') === GITHUB.clientSecret &&
      form.get('redirect_uri') === grant.redirectUri &&
      challenge === grant.challenge;
    if (!valid) {
      // GitHub refuses a code with `200 OK` and an OAuth error.
      return send(response, 200, { error: 'bad_verification_code' });
    }
    codes.delete(form.get('code') ?? '');
    const token = randomUUID();
    tokens.set(token, grant.account);
    return send(response, 200, { access_token: token, token_type: 'bearer', scope: 'user:email' });
  }

  const signedIn = tokens.get(bearer(request) ?? '');
  if (!signedIn) {
    return send(response, 401, { message: 'Requires authentication' });
  }
  if (route === 'GET /user') {
    return send(response, 200, { id: signedIn.id, login: signedIn.login, name: signedIn.name });
  }
  if (route === 'GET /user/emails') {
    return send(response, 200, [
      { email: signedIn.email, primary: true, verified: true, visibility: 'public' },
    ]);
  }
  return send(response, 404, { message: 'Not Found' });
}

// The admin API

const ALGORITHMS = ['ES256', 'ES384', 'EdDSA', 'PS256'];
/** Members that only private and symmetric keys have (RFC 7518 §6). */
const PRIVATE_MEMBERS = ['d', 'p', 'q', 'dp', 'dq', 'qi', 'oth', 'k'];
/** The sandbox tier's longest lifetime, in seconds. */
const SANDBOX_LIFETIME = 30 * 24 * 60 * 60;

const signingKey = createPublicKey(readFileSync(ADMIN_API.signingKeyFile));
const issuers = new Map<string, { issuer: Json; version: number }>();

/** Whether the request carries a valid admin token of the portal's. */
function authorized(request: IncomingMessage): boolean {
  try {
    const [header, claims, signature] = (bearer(request) ?? '').split('.');
    if (!header || !claims || !signature) {
      return false;
    }
    const signed = verify(
      'sha256',
      Buffer.from(`${header}.${claims}`),
      { key: signingKey, dsaEncoding: 'ieee-p1363' },
      Buffer.from(signature, 'base64url'),
    );
    const decode = (part: string) => JSON.parse(Buffer.from(part, 'base64url').toString('utf8'));
    const { alg, typ, kid } = decode(header);
    const { iss, aud, exp } = decode(claims);
    const audiences = Array.isArray(aud) ? aud : [aud];
    return (
      signed &&
      alg === 'ES256' &&
      typ === 'statuslist-admin+jwt' &&
      kid === ADMIN_API.keyId &&
      iss === ADMIN_API.principal &&
      audiences.includes(ADMIN_API.audience) &&
      exp > Date.now() / 1000
    );
  } catch {
    return false;
  }
}

function refuse(response: ServerResponse, status: number, error: string, description?: string) {
  return send(response, status, { error, error_description: description ?? error });
}

/** Why an expiry would be refused, if it would: the sandbox keeps issuers 30 days at most. */
function invalidExpiry(expiresAt: unknown): string | undefined {
  const now = Date.now() / 1000;
  const valid =
    typeof expiresAt === 'number' && expiresAt > now && expiresAt <= now + SANDBOX_LIFETIME;
  return valid ? undefined : 'expires_at must be in the future, within the tier lifetime';
}

/** Why a new issuer would be refused, if it would. */
function invalidIssuer(body: Json): string | undefined {
  const key = body.public_key;
  if (typeof body.display_name !== 'string' || !body.display_name.trim()) {
    return 'display_name is missing';
  }
  if (typeof body.owner !== 'string' || !/^github\|\d+$/.test(body.owner)) {
    return 'owner must be github|<id>';
  }
  if (!ALGORITHMS.includes(body.alg as string)) {
    return 'alg is not supported';
  }
  if (typeof key !== 'object' || key === null || PRIVATE_MEMBERS.some((name) => name in key)) {
    return 'public_key must be a public JWK';
  }
  return invalidExpiry(body.expires_at);
}

/** Why a change would be refused, if it would: it sets exactly one of the expiry or state. */
function invalidChange(change: Json): string | undefined {
  const keys = Object.keys(change);
  if (keys.length !== 1) {
    return 'set exactly one of expires_at or state';
  }
  if ('state' in change) {
    return ['active', 'suspended'].includes(change.state as string) ? undefined : 'unknown state';
  }
  return 'expires_at' in change ? invalidExpiry(change.expires_at) : 'unknown change';
}

/** The issuer as the admin API shows it now: an active issuer past its expiry is expired. */
function shown({ issuer }: { issuer: Json }): Json {
  const expired =
    issuer.state === 'active' &&
    typeof issuer.expires_at === 'number' &&
    issuer.expires_at <= Date.now() / 1000;
  return expired ? { ...issuer, state: 'expired' } : issuer;
}

function withVersion(
  response: ServerResponse,
  status: number,
  stored: { issuer: Json; version: number },
) {
  return send(response, status, shown(stored), { 'issuer-version': String(stored.version) });
}

function keyMaterial(key: unknown): string {
  const { kty, crv, x, y, n, e } = key as Json;
  return JSON.stringify({ kty, crv, x, y, n, e });
}

function createIssuer(body: Json): { issuer: Json; version: number } {
  const now = Math.floor(Date.now() / 1000);
  const issuer: Json = {
    id: randomUUID(),
    aggregation_id: randomUUID(),
    display_name: body.display_name,
    source: 'portal',
    tier: 'sandbox',
    state: 'active',
    owner: body.owner,
    created_at: now,
    expires_at: body.expires_at,
    keys: [
      {
        kid: randomUUID(),
        alg: body.alg,
        state: 'active',
        not_before: null,
        not_after: null,
        created_at: now,
        public_key: body.public_key,
      },
    ],
    usage: { lists: 0, lists_limit: 10, keys: 1, keys_limit: 2 },
  };
  const stored = { issuer, version: 1 };
  issuers.set(issuer.id as string, stored);
  return stored;
}

async function adminApi(request: IncomingMessage, response: ServerResponse) {
  const url = new URL(request.url ?? '/', ADMIN_API.url);
  if (request.method === 'GET' && url.pathname === '/health') {
    return send(response, 200, {});
  }
  if (request.method === 'GET' && url.pathname === '/e2e/issuers') {
    const held = [...issuers.values()].map((stored) => ({
      ...shown(stored),
      version: stored.version,
    }));
    return send(response, 200, held);
  }
  if (!authorized(request)) {
    return refuse(response, 401, 'invalid_token');
  }

  if (url.pathname === '/admin/v1/issuers') {
    if (request.method === 'GET') {
      const query = url.searchParams;
      const matching = [...issuers.values()]
        .map(shown)
        .filter((issuer) =>
          ['source', 'tier', 'state', 'owner'].every(
            (filter) => !query.has(filter) || issuer[filter] === query.get(filter),
          ),
        );
      const start = Number(query.get('cursor') ?? 0);
      const limit = Number(query.get('limit') ?? 50);
      const page = matching.slice(start, start + limit);
      const next = start + limit < matching.length ? String(start + limit) : null;
      return send(response, 200, { issuers: page, next_cursor: next });
    }
    if (request.method === 'POST') {
      const body = await readJson(request);
      const invalid = body ? invalidIssuer(body) : 'the body is not a JSON object';
      if (!body || invalid) {
        return refuse(response, 400, 'invalid_input', invalid);
      }
      const material = keyMaterial(body.public_key);
      const taken = [...issuers.values()].some(({ issuer }) =>
        (issuer.keys as Json[]).some((key) => keyMaterial(key.public_key) === material),
      );
      if (taken) {
        return refuse(response, 409, 'key_material_in_use');
      }
      return withVersion(response, 201, createIssuer(body));
    }
  }

  const id = url.pathname.match(/^\/admin\/v1\/issuers\/([^/]+)$/)?.[1];
  const stored = id && issuers.get(decodeURIComponent(id));
  if (!stored) {
    return refuse(response, 404, 'issuer_not_found');
  }
  if (request.method === 'GET') {
    return withVersion(response, 200, stored);
  }
  if (request.method === 'PATCH') {
    const version = request.headers['if-issuer-version'];
    if (version === undefined) {
      return refuse(response, 428, 'precondition_required');
    }
    if (version !== String(stored.version)) {
      return refuse(response, 412, 'precondition_failed');
    }
    const change = await readJson(request);
    const invalid = change ? invalidChange(change) : 'the body is not a JSON object';
    if (!change || invalid) {
      return refuse(response, 400, 'invalid_input', invalid);
    }
    const state = 'expires_at' in change ? 'active' : change.state;
    const updated = {
      issuer: { ...stored.issuer, ...change, state },
      version: stored.version + 1,
    };
    issuers.set(stored.issuer.id as string, updated);
    return withVersion(response, 200, updated);
  }
  return refuse(response, 405, 'method_not_allowed');
}

function serve(
  port: number,
  handle: (request: IncomingMessage, response: ServerResponse) => Promise<void>,
) {
  createServer((request, response) => {
    handle(request, response).catch((error: unknown) => {
      console.error(error);
      send(response, 500, { error: 'internal_error' });
    });
  }).listen(port, '127.0.0.1');
}

serve(GITHUB.port, github);
serve(ADMIN_API.port, adminApi);
console.log(`GitHub on ${GITHUB.url}, the admin API on ${ADMIN_API.url}`);
