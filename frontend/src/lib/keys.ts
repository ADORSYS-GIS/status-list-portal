/**
 * Instructions for developers: scripts that make a key pair, sign a management token and
 * publish a first status list. The scripts need only Node.js, whose Web Crypto API covers
 * every algorithm the Status List Server accepts.
 */

import type { Issuer, SigningAlgorithm } from './api';

/** Members that only private and symmetric keys have (RFC 7518 §6). */
const PRIVATE_MEMBERS = ['d', 'p', 'q', 'dp', 'dq', 'qi', 'oth', 'k'];

/** Seconds a management token stays valid, well within the server's limit. */
const TOKEN_LIFETIME = 120;

/** Web Crypto parameters, as JavaScript source, for each algorithm. */
const WEB_CRYPTO: Record<SigningAlgorithm, { key: string; sign: string }> = {
  ES256: {
    key: "{ name: 'ECDSA', namedCurve: 'P-256' }",
    sign: "{ name: 'ECDSA', hash: 'SHA-256' }",
  },
  ES384: {
    key: "{ name: 'ECDSA', namedCurve: 'P-384' }",
    sign: "{ name: 'ECDSA', hash: 'SHA-384' }",
  },
  EdDSA: {
    key: "{ name: 'Ed25519' }",
    sign: "{ name: 'Ed25519' }",
  },
  PS256: {
    key: "{ name: 'RSA-PSS', hash: 'SHA-256', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]) }",
    sign: "{ name: 'RSA-PSS', saltLength: 32 }",
  },
};

/**
 * Why `text` cannot be sent as a public key, or `null` when it can. The backend checks the
 * key again; this catches a pasted private key before it leaves the browser.
 */
export function publicKeyProblem(text: string): string | null {
  let key: unknown;
  try {
    key = JSON.parse(text);
  } catch {
    return 'This is not JSON. Paste the public key as a JSON Web Key.';
  }
  if (typeof key !== 'object' || key === null || Array.isArray(key)) {
    return 'Paste the public key as a JSON Web Key: a JSON object.';
  }
  if (PRIVATE_MEMBERS.some((member) => member in key)) {
    return 'This is a private key. Do not send it anywhere: make a new key pair and paste only its public key.';
  }
  return null;
}

/** A Node.js script that makes an `alg` key pair, saves the private key and prints the public one. */
export function keyPairScript(alg: SigningAlgorithm): string {
  return `// make-key.mjs: run \`node make-key.mjs\` (Node.js 22 or later).
import { writeFile } from 'node:fs/promises';

const { privateKey, publicKey } = await crypto.subtle.generateKey(
  ${WEB_CRYPTO[alg].key},
  true,
  ['sign', 'verify'],
);
// Keep private-key.json to yourself. It signs your issuer's tokens.
await writeFile('private-key.json', JSON.stringify(await crypto.subtle.exportKey('jwk', privateKey)), {
  mode: 0o600,
});
console.log(JSON.stringify(await crypto.subtle.exportKey('jwk', publicKey)));
`;
}

/** The key that signs the issuer's tokens: its newest active one. */
export function signingKey(issuer: Issuer) {
  return issuer.keys.find((key) => key.state === 'active') ?? issuer.keys[0];
}

/** A Node.js script that signs a management token for `issuer` with private-key.json. */
export function tokenScript(issuer: Issuer): string {
  const key = signingKey(issuer);
  const alg = key?.alg ?? 'ES256';
  const header = { alg, typ: 'statuslist-management+jwt', kid: key?.kid ?? '' };
  return `// mint-token.mjs: run \`node mint-token.mjs\` (Node.js 22 or later).
import { readFile } from 'node:fs/promises';

const jwk = JSON.parse(await readFile('private-key.json', 'utf8'));
const key = await crypto.subtle.importKey('jwk', jwk, ${WEB_CRYPTO[alg].key}, false, ['sign']);
const now = Math.floor(Date.now() / 1000);
const header = ${JSON.stringify(header)};
const claims = { iss: ${JSON.stringify(issuer.id)}, aud: ${JSON.stringify(issuer.audience)}, iat: now, exp: now + ${TOKEN_LIFETIME} };
const encode = (part) => Buffer.from(JSON.stringify(part)).toString('base64url');
const input = \`\${encode(header)}.\${encode(claims)}\`;
const signature = await crypto.subtle.sign(${WEB_CRYPTO[alg].sign}, key, new TextEncoder().encode(input));
console.log(\`\${input}.\${Buffer.from(signature).toString('base64url')}\`);
`;
}

/** Shell commands that publish a status list with one valid entry, as `issuer`. */
export function publishCommands(issuer: Issuer): string {
  return `TOKEN=$(node mint-token.mjs)
LIST_ID=$(node -e "console.log(crypto.randomUUID())")
curl -X PUT "${issuer.management_url}/api/v1/status-lists/$LIST_ID/statuses" \\
  -H "Authorization: Bearer $TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '{"statuses": [{"index": 0, "status": "VALID"}]}'
`;
}
