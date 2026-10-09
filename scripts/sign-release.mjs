import { spawnSync } from 'node:child_process';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { existsSync, readFileSync, unlinkSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const REVOKED_KEY_ID = '2340F69B968E764E';

export function publicKeyId(encoded) {
  const lines = Buffer.from(encoded, 'base64').toString('utf8').trim().split(/\r?\n/);
  const key = Buffer.from(lines.at(-1) ?? '', 'base64');
  if (key.length !== 42 || key.subarray(0, 2).toString() !== 'Ed') {
    throw new Error('Invalid updater public key.');
  }
  return key.readBigUInt64LE(2).toString(16).toUpperCase().padStart(16, '0');
}

export function signingEnvironment(env, publicKey) {
  if (publicKeyId(publicKey) === REVOKED_KEY_ID) {
    throw new Error('The exposed updater key is revoked. Rotate it before signing.');
  }
  const key = env.TAURI_SIGNING_PRIVATE_KEY;
  const keyPath = env.TAURI_SIGNING_PRIVATE_KEY_PATH;
  if ((!key && !keyPath) || (key && keyPath)) {
    throw new Error('Set exactly one of TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH.');
  }
  if (!env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD?.trim()) {
    throw new Error('A nonempty TAURI_SIGNING_PRIVATE_KEY_PASSWORD is required.');
  }
  const result = { ...env };
  // Empty optional variables still reach clap as values. Omit them entirely.
  if (!key) delete result.TAURI_SIGNING_PRIVATE_KEY;
  if (!keyPath) delete result.TAURI_SIGNING_PRIVATE_KEY_PATH;
  return result;
}

export function verifyUpdaterSignature(encodedPublicKey, encodedSignature, artifact) {
  const id = publicKeyId(encodedPublicKey);
  const publicLines = Buffer.from(encodedPublicKey, 'base64').toString('utf8').trim().split(/\r?\n/);
  const publicBytes = Buffer.from(publicLines.at(-1), 'base64');
  const lines = Buffer.from(encodedSignature, 'base64').toString('utf8').trim().split(/\r?\n/);
  const signature = Buffer.from(lines[1] ?? '', 'base64');
  const commentSignature = Buffer.from(lines[3] ?? '', 'base64');
  if (lines.length !== 4 || !lines[0].startsWith('untrusted comment: ')
      || !lines[2].startsWith('trusted comment: ') || signature.length !== 74
      || !['ED', 'Ed'].includes(signature.subarray(0, 2).toString())
      || commentSignature.length !== 64
      || signature.readBigUInt64LE(2).toString(16).toUpperCase().padStart(16, '0') !== id) {
    throw new Error('Malformed updater signature or mismatched public key.');
  }
  const key = createPublicKey({ key: Buffer.concat([
    Buffer.from('302a300506032b6570032100', 'hex'), publicBytes.subarray(10),
  ]), format: 'der', type: 'spki' });
  const data = signature.subarray(0, 2).toString() === 'ED'
    ? createHash('blake2b512').update(artifact).digest() : artifact;
  const rawSignature = signature.subarray(10);
  const comment = Buffer.from(lines[2].slice('trusted comment: '.length).trim(), 'utf8');
  if (!verify(null, data, key, rawSignature)
      || !verify(null, Buffer.concat([rawSignature, comment]), key, commentSignature)) {
    throw new Error('Updater signature verification failed. Do not publish this signature.');
  }
}

export function signRelease(version, installer) {
  if (!/^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(version ?? '')) {
    throw new Error('Provide an explicit release version, for example: node scripts/sign-release.mjs 0.2.3');
  }
  const config = JSON.parse(readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  const env = signingEnvironment(process.env, config.plugins.updater.pubkey);
  const target = installer ? path.resolve(installer) : path.join(root, `src-tauri/target/release/bundle/nsis/Typr_${version}_x64-setup.exe`);
  if (!existsSync(target)) throw new Error('Release installer not found. Build it before signing.');
  const cli = path.join(root, 'node_modules/@tauri-apps/cli/tauri.js');
  // No shell interpolation and no credentials in arguments or console output.
  const result = spawnSync(process.execPath, [cli, 'signer', 'sign', target], {
    cwd: root, env, encoding: 'utf8', windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  });
  if (result.error || result.status !== 0) {
    throw new Error('Signing failed. Check the private key and password in your secure credential store.');
  }
  // Verify the artifact AND trusted comment, not just the key identifier.
  const signature = readFileSync(`${target}.sig`, 'utf8').trim();
  try {
    verifyUpdaterSignature(config.plugins.updater.pubkey, signature, readFileSync(target));
  } catch {
    unlinkSync(`${target}.sig`);
    throw new Error('Signature does not verify against the configured updater public key. Do not publish it.');
  }
  console.log(`Signed Typr ${version} with the current updater key.`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { signRelease(process.argv[2], process.argv[3]); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
