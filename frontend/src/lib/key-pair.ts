/**
 * Key pairs made in the browser, for developers without one. The private key reaches the
 * developer only as files to save: the portal never needs it, so nothing here sends it.
 */

const ES256 = { name: 'ECDSA', namedCurve: 'P-256' };

export interface KeyPair {
  /** The public key as the registration form takes it: a JWK with its public members only. */
  publicJwk: string;
  /** The private key as a JWK, the form `mint-token.mjs` reads from `private-key.json`. */
  privateJwk: string;
  /** The private key as PKCS#8 PEM, for tools that take PEM files. */
  privatePem: string;
}

/** Makes an ES256 key pair, the server's default algorithm. */
export async function makeKeyPair(): Promise<KeyPair> {
  const { privateKey, publicKey } = await crypto.subtle.generateKey(ES256, true, [
    'sign',
    'verify',
  ]);
  const [{ kty, crv, x, y }, privateJwk, pkcs8] = await Promise.all([
    crypto.subtle.exportKey('jwk', publicKey),
    crypto.subtle.exportKey('jwk', privateKey),
    crypto.subtle.exportKey('pkcs8', privateKey),
  ]);
  return {
    publicJwk: JSON.stringify({ kty, crv, x, y }),
    privateJwk: JSON.stringify(privateJwk),
    privatePem: pem('PRIVATE KEY', pkcs8),
  };
}

function pem(label: string, der: ArrayBuffer): string {
  const base64 = btoa(String.fromCharCode(...new Uint8Array(der)));
  const lines = base64.match(/.{1,64}/g) ?? [];
  return `-----BEGIN ${label}-----\n${lines.join('\n')}\n-----END ${label}-----\n`;
}
