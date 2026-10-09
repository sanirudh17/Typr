import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { writeFileSync, unlinkSync } from 'node:fs';
import { createHash, generateKeyPairSync, sign, randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { publicKeyId, signingEnvironment, verifyUpdaterSignature, REVOKED_KEY_ID } from './sign-release.mjs';

function publicKey(id) {
  const bytes = Buffer.alloc(42);
  bytes.write('Ed');
  bytes.writeBigUInt64LE(BigInt(`0x${id}`), 2);
  return Buffer.from(`untrusted comment: test public key\n${bytes.toString('base64')}\n`).toString('base64');
}
const current = publicKey('0000000000000123');
test('rejects revoked public key even when credentials are provided', () => {
  assert.throws(() => signingEnvironment({ TAURI_SIGNING_PRIVATE_KEY: 'test', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'test' }, publicKey(REVOKED_KEY_ID)), /revoked/);
});
test('requires private credentials and a nonempty password', () => {
  assert.throws(() => signingEnvironment({}, current), /exactly one/);
  assert.throws(() => signingEnvironment({ TAURI_SIGNING_PRIVATE_KEY: 'test' }, current), /nonempty/);
  assert.throws(() => signingEnvironment({ TAURI_SIGNING_PRIVATE_KEY: 'test', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '  ' }, current), /nonempty/);
  assert.throws(() => signingEnvironment({ TAURI_SIGNING_PRIVATE_KEY: 'test', TAURI_SIGNING_PRIVATE_KEY_PATH: 'test', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'test' }, current), /exactly one/);
});
test('accepts either injected key content or a secure external key path', () => {
  for (const key of [{ TAURI_SIGNING_PRIVATE_KEY: 'test' }, { TAURI_SIGNING_PRIVATE_KEY_PATH: 'external.key' }]) {
    const env = { ...key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'test', PATH: 'unchanged' };
    assert.deepEqual(signingEnvironment(env, current), env);
  }
});
test('validates the actual minisign public key payload', () => {
  assert.equal(publicKeyId(current), '0000000000000123');
  assert.throws(() => publicKeyId('invalid'), /Invalid/);
});
test('omits empty optional credentials instead of passing ambiguous values to the signer', () => {
  const env = { TAURI_SIGNING_PRIVATE_KEY: '', TAURI_SIGNING_PRIVATE_KEY_PATH: 'external.key', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'test' };
  assert.deepEqual(signingEnvironment(env, current), { TAURI_SIGNING_PRIVATE_KEY_PATH: 'external.key', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'test' });
});
test('repository scanner detects encoded secrets without printing them', () => {
  // Deliberately invalid dummy data, never a real private key.
  const dummy = Buffer.from('untrusted comment: ' + 'minisign encrypted secret key\n' + 'dummy'.repeat(40)).toString('base64');
  const fixture = new URL(`secret-guard-${randomUUID()}.txt`, import.meta.url);
  const root = fileURLToPath(new URL('../', import.meta.url));
  try {
    writeFileSync(fixture, dummy);
    const result = spawnSync(process.execPath, ['scripts/check-secrets.mjs'], { cwd: root, encoding: 'utf8', windowsHide: true });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /Embedded signing secret: scripts\/secret-guard-/);
    assert.ok(!result.stderr.includes(dummy));
    assert.ok(!result.stderr.includes('minisign encrypted secret key'));
  } finally { unlinkSync(fixture); }
});

test('repository scanner checks Windows UTF-16 exports without exposing dummy credentials', () => {
  const header = 'untrusted comment: ' + 'minisign encrypted secret key';
  const dummy = Buffer.from(header + '\n' + 'dummy'.repeat(40)).toString('base64');
  const fixture = new URL(`secret-guard-${randomUUID()}.txt`, import.meta.url);
  const root = fileURLToPath(new URL('../', import.meta.url));
  try {
    for (const text of [header + '\ndummy', dummy]) {
      for (const bigEndian of [false, true]) {
        const bytes = Buffer.from(text, 'utf16le');
        writeFileSync(fixture, Buffer.concat([
          Buffer.from(bigEndian ? [0xfe, 0xff] : [0xff, 0xfe]),
          bigEndian ? bytes.swap16() : bytes,
        ]));
        const result = spawnSync(process.execPath, ['scripts/check-secrets.mjs'], { cwd: root, encoding: 'utf8', windowsHide: true });
        assert.equal(result.status, 1);
        assert.match(result.stderr, /Embedded signing secret: scripts\/secret-guard-/);
        assert.ok(!result.stderr.includes(dummy));
        assert.ok(!result.stderr.includes(header));
      }
    }
  } finally { unlinkSync(fixture); }
});

test('verifies both the artifact and trusted comment, rejecting tampering and wrong keys', () => {
  // Ephemeral in-memory test keys; no private key is serialized or written.
  const { publicKey: publicObject, privateKey } = generateKeyPairSync('ed25519');
  const payload = publicObject.export({ format: 'der', type: 'spki' }).subarray(-32);
  const publicBytes = Buffer.alloc(42);
  publicBytes.write('Ed'); publicBytes.writeBigUInt64LE(123n, 2); payload.copy(publicBytes, 10);
  const publicEncoded = Buffer.from(`untrusted comment: test public key\n${publicBytes.toString('base64')}\n`).toString('base64');
  const artifact = Buffer.from('test installer data');
  for (const algorithm of ['Ed', 'ED']) {
    const raw = sign(null, algorithm === 'ED' ? createHash('blake2b512').update(artifact).digest() : artifact, privateKey);
    const signature = Buffer.alloc(74);
    signature.write(algorithm); signature.writeBigUInt64LE(123n, 2); raw.copy(signature, 10);
    const comment = 'timestamp:123';
    const global = sign(null, Buffer.concat([raw, Buffer.from(comment)]), privateKey);
    const envelope = c => Buffer.from(`untrusted comment: test signature\n${signature.toString('base64')}\ntrusted comment: ${c}\n${global.toString('base64')}\n`).toString('base64');
    assert.doesNotThrow(() => verifyUpdaterSignature(publicEncoded, envelope(comment), artifact));
    assert.throws(() => verifyUpdaterSignature(publicEncoded, envelope(comment), Buffer.from('changed installer')), /verification failed/);
    assert.throws(() => verifyUpdaterSignature(publicEncoded, envelope('changed comment'), artifact), /verification failed/);
    assert.throws(() => verifyUpdaterSignature(publicKey('000000000000007B'), envelope(comment), artifact), /verification failed/);
    assert.throws(() => verifyUpdaterSignature(publicKey('000000000000FFFF'), envelope(comment), artifact), /mismatched/);
  }
});
