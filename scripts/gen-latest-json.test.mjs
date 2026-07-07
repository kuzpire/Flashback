import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import assert from 'node:assert/strict';

const dir = mkdtempSync(join(tmpdir(), 'flb-'));
const sig = join(dir, 'setup.sig');
writeFileSync(sig, 'SIGNATURE_CONTENT\n');

const out = execFileSync('node', ['scripts/gen-latest-json.mjs', '1.4.0', sig], {
  env: { ...process.env, RELEASE_NOTES: 'Hello notes' },
  encoding: 'utf8'
});
const m = JSON.parse(out);

assert.equal(m.version, '1.4.0');
assert.equal(m.notes, 'Hello notes');
assert.equal(m.platforms['windows-x86_64'].signature, 'SIGNATURE_CONTENT');
assert.equal(
  m.platforms['windows-x86_64'].url,
  'https://github.com/kuzpire/Flashback/releases/download/v1.4.0/Flashback_1.4.0_x64-setup.exe'
);
console.log('gen-latest-json: ok');
