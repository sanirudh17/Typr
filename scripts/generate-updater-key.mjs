import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Called by rotate-updater-key.ps1 after the external directory ACL is applied.
// Use the in-process CLI API: passwords never appear in the OS command line.
const require = createRequire(import.meta.url);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
try {
  const password = process.env.TYPR_NEW_UPDATER_PASSWORD;
  delete process.env.TYPR_NEW_UPDATER_PASSWORD;
  if (!password || !process.argv[2]) throw new Error('Missing generation credentials.');
  const cli = require(path.join(root, 'node_modules/@tauri-apps/cli/main.js'));
  await cli.run(['signer', 'generate', '--ci', '-p', password, '-w', process.argv[2]], 'typr-key-generator');
} catch {
  console.error('Updater key generation failed. No credentials were printed.');
  process.exitCode = 1;
}
