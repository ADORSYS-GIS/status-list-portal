import { readFile } from 'node:fs/promises';

import openapiTS, { astToString } from 'openapi-typescript';
import { expect, test } from 'vitest';

/** The backend's OpenAPI document, which a snapshot test of the backend keeps current. */
const SNAPSHOT = new URL(
  '../../../../src/snapshots/status_list_portal__openapi__tests__openapi_snapshot.snap',
  import.meta.url,
);

const HEADER = `/**
 * Generated from the backend's OpenAPI document. Do not edit.
 *
 * Run \`bun run test -u\` to update it.
 */

`;

async function generateSchema(): Promise<string> {
  const snapshot = await readFile(SNAPSHOT, 'utf8');
  // The snapshot starts with a YAML header; the document is the JSON after it.
  const document = JSON.parse(snapshot.slice(snapshot.indexOf('{')));
  const ast = await openapiTS(document, { enumValues: true });
  return HEADER + astToString(ast);
}

test('schema.ts matches the backend API', async () => {
  await expect(await generateSchema()).toMatchFileSnapshot('./schema.ts');
});
