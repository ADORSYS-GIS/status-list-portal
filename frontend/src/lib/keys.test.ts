import { execFile } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';

import { afterEach, beforeEach, expect, test } from 'vitest';

import type { Issuer, SigningAlgorithm } from './api';
import { keyPairScript, publicKeyProblem, publishCommands, tokenScript } from './keys';

const run = promisify(execFile);

/** How the server checks each algorithm's signatures. */
const VERIFY: Record<SigningAlgorithm, { key: AlgorithmIdentifier; verify: AlgorithmIdentifier }> =
  {
    ES256: {
      key: { name: 'ECDSA', namedCurve: 'P-256' } as EcKeyImportParams,
      verify: { name: 'ECDSA', hash: 'SHA-256' } as EcdsaParams,
    },
    ES384: {
      key: { name: 'ECDSA', namedCurve: 'P-384' } as EcKeyImportParams,
      verify: { name: 'ECDSA', hash: 'SHA-384' } as EcdsaParams,
    },
    EdDSA: { key: { name: 'Ed25519' }, verify: { name: 'Ed25519' } },
    PS256: {
      key: { name: 'RSA-PSS', hash: 'SHA-256' } as RsaHashedImportParams,
      verify: { name: 'RSA-PSS', saltLength: 32 } as RsaPssParams,
    },
  };

let dir: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'portal-keys-'));
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

/** Runs a script on the tests' runtime: Bun, and in CI also Node.js, which developers use. */
async function runScript(name: string, source: string): Promise<string> {
  await writeFile(join(dir, name), source);
  const { stdout } = await run(process.execPath, [name], { cwd: dir });
  return stdout.trim();
}

function issuer(alg: SigningAlgorithm): Issuer {
  return {
    id: '6f1c2a9e-0b7d-4c1e-9a53-2f8d7e4b1c00',
    display_name: 'Acme Wallet Issuer',
    state: 'active',
    created_at: null,
    expires_at: null,
    keys: [{ kid: 'the-kid', alg, state: 'active' }],
    management_url: 'https://status.example.com',
    audience: 'https://status.example.com',
  };
}

function decodePart(part: string): Record<string, unknown> {
  return JSON.parse(Buffer.from(part, 'base64url').toString('utf8'));
}

test.each(['ES256', 'ES384', 'EdDSA', 'PS256'] as const)(
  'the %s scripts make a key and a token the server accepts',
  async (alg) => {
    const publicJwk = JSON.parse(await runScript('make-key.mjs', keyPairScript(alg)));
    expect(publicKeyProblem(JSON.stringify(publicJwk))).toBeNull();

    const token = await runScript('mint-token.mjs', tokenScript(issuer(alg)));

    const [header, claims, signature] = token.split('.');
    expect(decodePart(header)).toEqual({ alg, typ: 'statuslist-management+jwt', kid: 'the-kid' });
    const { iss, aud, iat, exp } = decodePart(claims);
    expect({ iss, aud }).toEqual({ iss: issuer(alg).id, aud: 'https://status.example.com' });
    expect(Number(exp) - Number(iat)).toBe(120);
    const key = await crypto.subtle.importKey('jwk', publicJwk, VERIFY[alg].key, false, ['verify']);
    const signed = new TextEncoder().encode(`${header}.${claims}`);
    const valid = await crypto.subtle.verify(
      VERIFY[alg].verify,
      key,
      Buffer.from(signature, 'base64url'),
      signed,
    );
    expect(valid).toBe(true);
  },
);

test('private keys are refused before they are sent', () => {
  for (const member of ['d', 'p', 'q', 'dp', 'dq', 'qi', 'oth', 'k']) {
    const key = JSON.stringify({ kty: 'EC', crv: 'P-256', x: 'AAAA', y: 'BBBB', [member]: 'c2Vj' });
    expect(publicKeyProblem(key)).toMatch(/private key/);
  }
});

test('text that is not a JSON object is refused', () => {
  expect(publicKeyProblem('ssh-ed25519 AAAA')).toMatch(/not JSON/);
  expect(publicKeyProblem('[1, 2]')).toMatch(/JSON object/);
});

test('a private key inside a key set is refused before it is sent', () => {
  const keySet = JSON.stringify({
    keys: [{ kty: 'EC', crv: 'P-256', x: 'AAAA', y: 'BBBB', d: 'c2Vj' }],
  });

  expect(publicKeyProblem(keySet)).toMatch(/private key/);
});

test('only one key is accepted', () => {
  const keySet = JSON.stringify({ keys: [{ kty: 'EC', crv: 'P-256', x: 'AAAA', y: 'BBBB' }] });

  expect(publicKeyProblem(keySet)).toMatch(/one public key/);
  expect(publicKeyProblem('{"kty": 1}')).toMatch(/one public key/);
});

test('publishing goes to the management API with a token', () => {
  const commands = publishCommands(issuer('ES256'));

  expect(commands).toContain(
    'curl -X PUT "https://status.example.com/api/v1/status-lists/$LIST_ID/statuses"',
  );
  expect(commands).toContain('Authorization: Bearer $TOKEN');
});
