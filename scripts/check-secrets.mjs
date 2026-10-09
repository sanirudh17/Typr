import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const files = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], {
  cwd: root, encoding: 'utf8', windowsHide: true,
}).split('\0').filter(Boolean);
let failures = 0;
for (const file of new Set(files)) {
  if (/\.(?:key|dpapi|pfx|p12)$/.test(file) || /(?:^|\/)\.env(?:\.|$)/.test(file)) {
    console.error(`Private credential file: ${file}`);
    failures++;
    continue;
  }
  if (/\.(?:png|ico|icns|wav|mp3|pdf|woff2?|lock)$/.test(file)) continue;
  const bytes = readFileSync(path.join(root, file));
  // Windows PowerShell exports UTF-16 text. Its NUL bytes must not make a
  // credential-containing patch or script look like an opaque binary asset.
  let decoded;
  if (bytes[0] === 0xff && bytes[1] === 0xfe) {
    decoded = bytes.subarray(2).toString('utf16le');
  } else if (bytes[0] === 0xfe && bytes[1] === 0xff) {
    const textBytes = Buffer.from(bytes.subarray(2));
    if (textBytes.length % 2) { console.error(`Malformed UTF-16 file: ${file}`); failures++; continue; }
    decoded = textBytes.swap16().toString('utf16le');
  } else {
    if (bytes.includes(0)) continue;
    decoded = bytes.toString('utf8');
  }
  const lines = decoded.split(/\r?\n/);
  for (const [index, line] of lines.entries()) {
    const encoded = line.match(/[A-Za-z0-9+/]{80,}={0,2}/g) ?? [];
    const secret = /^\s*untrusted comment: (?:minisign )?(?:encrypted )?secret key/i.test(line)
      || encoded.some(value => /^\s*untrusted comment: (?:minisign )?(?:encrypted )?secret key/i.test(Buffer.from(value, 'base64').toString('utf8')))
      || /TAURI_SIGNING_PRIVATE_KEY\s*[:=]\s*["'][A-Za-z0-9+/]{40,}/.test(line);
    if (secret) {
      // Never print the offending line: the diagnostic must not become a leak.
      console.error(`Embedded signing secret: ${file}:${index + 1}`);
      failures++;
    }
  }
}
if (failures) process.exitCode = 1;
else console.log(`No updater signing secrets found in ${new Set(files).size} repository files.`);
