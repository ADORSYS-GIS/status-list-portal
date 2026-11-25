/**
 * Key pairs made in the browser, for developers without one. The private key reaches the
 * developer only as files to save: the portal never needs it, so nothing here sends it.
 */

import type { SigningAlgorithm } from './api';

const ALGORITHMS: Record<SigningAlgorithm, EcKeyGenParams | RsaHashedKeyGenParams | Algorithm> = {
  ES256: { name: 'ECDSA', namedCurve: 'P-256' },
  ES384: { name: 'ECDSA', namedCurve: 'P-384' },
  EdDSA: { name: 'Ed25519' },
  PS256: {
    name: 'RSA-PSS',
    hash: 'SHA-256',
    modulusLength: 2048,
    publicExponent: new Uint8Array([1, 0, 1]),
  },
};

export interface KeyPair {
  /** The public key as the registration form takes it: a JWK with its public members only. */
  publicJwk: string;
  /** The private key as a JWK, the form `mint-token.mjs` reads from `private-key.json`. */
  privateJwk: string;
  /** The private key as PKCS#8 PEM, for tools that take PEM files. */
  privatePem: string;
}

/** Makes a key pair for any algorithm the server accepts, defaulting to ES256. */
export async function makeKeyPair(alg: SigningAlgorithm = 'ES256'): Promise<KeyPair> {
  const { privateKey, publicKey } = (await crypto.subtle.generateKey(ALGORITHMS[alg], true, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  const [exportedPublic, privateJwk, pkcs8] = await Promise.all([
    crypto.subtle.exportKey('jwk', publicKey),
    crypto.subtle.exportKey('jwk', privateKey),
    crypto.subtle.exportKey('pkcs8', privateKey),
  ]);
  const { kty, crv, x, y, n, e } = exportedPublic;
  const publicMembers =
    alg === 'PS256' ? { kty, n, e } : alg === 'EdDSA' ? { kty, crv, x } : { kty, crv, x, y };
  return {
    publicJwk: JSON.stringify(publicMembers),
    privateJwk: JSON.stringify(privateJwk),
    privatePem: pem('PRIVATE KEY', pkcs8),
  };
}

function pem(label: string, der: ArrayBuffer): string {
  const base64 = btoa(String.fromCharCode(...new Uint8Array(der)));
  const lines = base64.match(/.{1,64}/g) ?? [];
  return `-----BEGIN ${label}-----\n${lines.join('\n')}\n-----END ${label}-----\n`;
}
