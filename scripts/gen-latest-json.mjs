import { readFileSync } from 'node:fs';

const [version, sigPath] = process.argv.slice(2);
if (!version || !sigPath) {
  console.error('usage: gen-latest-json.mjs <version> <sigPath>');
  process.exit(1);
}

const signature = readFileSync(sigPath, 'utf8').trim();
const url = `https://github.com/kuzpire/Flashback/releases/download/v${version}/Flashback_${version}_x64-setup.exe`;

const manifest = {
  version,
  notes: process.env.RELEASE_NOTES ?? '',
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': { signature, url }
  }
};

process.stdout.write(JSON.stringify(manifest, null, 2));
