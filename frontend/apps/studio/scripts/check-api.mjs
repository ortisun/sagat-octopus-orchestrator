// Fails when src/api/schema.d.ts is not what the OpenAPI contract generates: the contract is
// the source, the types follow it (the SDKs do the same with worker.proto).
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const dir = mkdtempSync(join(tmpdir(), 'studio-api-'));
try {
  const out = join(dir, 'schema.d.ts');
  execFileSync('npx', ['openapi-typescript', '../../../specs/001-platform-foundation/contracts/openapi.yaml', '-o', out], { stdio: 'ignore' });
  if (readFileSync(out, 'utf8') !== readFileSync('src/api/schema.d.ts', 'utf8')) {
    console.error('src/api/schema.d.ts is behind the OpenAPI contract: run `npm run api`.');
    process.exit(1);
  }
  console.log('src/api/schema.d.ts matches the OpenAPI contract.');
} finally {
  rmSync(dir, { recursive: true, force: true });
}
