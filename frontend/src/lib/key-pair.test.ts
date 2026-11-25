import { expect, test } from 'vitest';

import type { SigningAlgorithm } from './api';
import { makeKeyPair } from './key-pair.ts';

const ALGORITHMS: Record<
  SigningAlgorithm,
  { key: AlgorithmIdentifier; sign: AlgorithmIdentifier }
> = {
  ES256: {
    key: { name: 'ECDSA', namedCurve: 'P-256' } as EcKeyImportParams,
    sign: { name: 'ECDSA', hash: 'SHA-256' } as EcdsaParams,
  },
  ES384: {
    key: { name: 'ECDSA', namedCurve: 'P-384' } as EcKeyImportParams,
    sign: { name: 'ECDSA', hash: 'SHA-384' } as EcdsaParams,
  },
  EdDSA: { key: { name: 'Ed25519' }, sign: { name: 'Ed25519' } },
  PS256: {
    key: { name: 'RSA-PSS', hash: 'SHA-256' } as RsaHashedImportParams,
    sign: { name: 'RSA-PSS', saltLength: 32 } as RsaPssParams,
  },
};

test('the public key holds its public members only', async () => {
  const { publicJwk } = await makeKeyPair();

  expect(Object.keys(JSON.parse(publicJwk)).sort()).toEqual(['crv', 'kty', 'x', 'y']);
});

test.each(['ES256', 'ES384', 'EdDSA', 'PS256'] as const)(
  '%s private key files sign what the public key verifies',
  async (alg) => {
    const pair = await makeKeyPair(alg);
    expect(pair.privatePem).toMatch(
      /^-----BEGIN PRIVATE KEY-----\n([A-Za-z0-9+/=]{1,64}\n)+-----END PRIVATE KEY-----\n$/,
    );
    const der = Uint8Array.from(atob(pair.privatePem.replace(/-----[^-]+-----|\s/g, '')), (c) =>
      c.charCodeAt(0),
    );
    const signers = await Promise.all([
      crypto.subtle.importKey('jwk', JSON.parse(pair.privateJwk), ALGORITHMS[alg].key, false, [
        'sign',
      ]),
      crypto.subtle.importKey('pkcs8', der, ALGORITHMS[alg].key, false, ['sign']),
    ]);
    const verifier = await crypto.subtle.importKey(
      'jwk',
      JSON.parse(pair.publicJwk),
      ALGORITHMS[alg].key,
      false,
      ['verify'],
    );
    const data = new TextEncoder().encode('a management token');

    for (const signer of signers) {
      const signature = await crypto.subtle.sign(ALGORITHMS[alg].sign, signer, data);
      expect(await crypto.subtle.verify(ALGORITHMS[alg].sign, verifier, signature, data)).toBe(
        true,
      );
    }
  },
);
