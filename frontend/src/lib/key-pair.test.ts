import { expect, test } from 'vitest';

import { makeKeyPair } from './key-pair.ts';

const ES256 = { name: 'ECDSA', namedCurve: 'P-256' };
const SIGNING = { name: 'ECDSA', hash: 'SHA-256' };

test('the public key holds its public members only', async () => {
  const { publicJwk } = await makeKeyPair();

  expect(Object.keys(JSON.parse(publicJwk)).sort()).toEqual(['crv', 'kty', 'x', 'y']);
});

test('both private key files sign what the public key verifies', async () => {
  const pair = await makeKeyPair();
  expect(pair.privatePem).toMatch(
    /^-----BEGIN PRIVATE KEY-----\n([A-Za-z0-9+/=]{1,64}\n)+-----END PRIVATE KEY-----\n$/,
  );
  const der = Uint8Array.from(atob(pair.privatePem.replace(/-----[^-]+-----|\s/g, '')), (c) =>
    c.charCodeAt(0),
  );
  const signers = await Promise.all([
    crypto.subtle.importKey('jwk', JSON.parse(pair.privateJwk), ES256, false, ['sign']),
    crypto.subtle.importKey('pkcs8', der, ES256, false, ['sign']),
  ]);
  const verifier = await crypto.subtle.importKey('jwk', JSON.parse(pair.publicJwk), ES256, false, [
    'verify',
  ]);
  const data = new TextEncoder().encode('a management token');

  for (const signer of signers) {
    const signature = await crypto.subtle.sign(SIGNING, signer, data);
    expect(await crypto.subtle.verify(SIGNING, verifier, signature, data)).toBe(true);
  }
});
