import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { verifyUpdaterSignature } from './sign-release.mjs';

const root = path.resolve(import.meta.dirname, '..');
process.chdir(root);
const version = process.argv[2];
assert.match(version ?? '', /^\d+\.\d+\.\d+$/, 'Provide an explicit release version');
const tag = `v${version}`;
const nsis = path.join(root, 'src-tauri/target/release/bundle/nsis');
const msi = path.join(root, 'src-tauri/target/release/bundle/msi');
const paths = [
  path.join(nsis, `Typr_${version}_x64-setup.exe`),
  path.join(msi, `Typr_${version}_x64_en-US.msi`),
  path.join(nsis, `Typr_${version}_x64-setup.exe.sig`),
  path.join(nsis, 'latest.json'),
  path.join(nsis, 'SHA256SUMS.txt'),
];
const digest = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const shell = (command, args) => execFileSync(command, args, { cwd: root, windowsHide: true, encoding: 'utf8' }).trim();
const config = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'));
assert.equal(config.version, version);
const pkg = JSON.parse(fs.readFileSync('package.json', 'utf8'));
const lock = JSON.parse(fs.readFileSync('package-lock.json', 'utf8'));
assert.equal(pkg.version, version); assert.equal(lock.version, version); assert.equal(lock.packages[''].version, version);
for (const file of ['src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']) assert.ok(fs.readFileSync(file, 'utf8').includes('name = '+JSON.stringify('typr')+'\nversion = '+JSON.stringify(version)) || fs.readFileSync(file, 'utf8').includes('name = '+JSON.stringify('typr')+'\r\nversion = '+JSON.stringify(version)), 'Rust version mismatch');
assert.equal(shell('git', ['branch', '--show-current']), 'main');
assert.equal(shell('git', ['rev-parse', 'HEAD']), shell('git', ['rev-parse', `${tag}^{commit}`]));
assert.equal(shell('git', ['diff', '--name-only', 'HEAD']), '');
const notes = fs.readFileSync(`release-notes-${tag}.md`, 'utf8').trim();
assert.doesNotMatch(notes, /[^\x00-\x7F]/);
for (const file of paths) assert.ok(fs.statSync(file).size > 0, `Missing asset: ${file}`);
assert.ok(fs.statSync(paths[0]).size > 1_000_000);
assert.ok(fs.statSync(paths[1]).size > 1_000_000);
assert.equal(fs.readFileSync(paths[0]).subarray(0, 2).toString(), 'MZ');
assert.equal(fs.readFileSync(paths[1]).subarray(0, 8).toString('hex'), 'd0cf11e0a1b11ae1');
const signature = fs.readFileSync(paths[2], 'utf8').trim();
verifyUpdaterSignature(config.plugins.updater.pubkey, signature, fs.readFileSync(paths[0]));
const manifestBytes = fs.readFileSync(paths[3]);
assert.notEqual(manifestBytes.subarray(0, 3).toString('hex'), 'efbbbf');
const manifest = JSON.parse(manifestBytes);
assert.equal(manifest.version, version);
assert.equal(manifest.notes, notes);
assert.ok(Number.isFinite(Date.parse(manifest.pub_date)));
assert.equal(manifest.platforms['windows-x86_64'].signature, signature);
assert.equal(manifest.platforms['windows-x86_64'].url,
  `https://github.com/sanirudh17/Typr/releases/download/${tag}/${path.basename(paths[0])}`);
const checksums = fs.readFileSync(paths[4], 'utf8').trim().split(/\r?\n/);
assert.equal(checksums.length, paths.length - 1);
for (const file of paths.slice(0, -1)) {
  assert.ok(checksums.includes(`${digest(file)}  ${path.basename(file)}`), `Checksum mismatch: ${file}`);
}
console.log('Local release verified: main/tag, versions, installers, cryptographic signature, updater manifest, checksums, and icon-free notes.');
if (process.argv.includes('--remote')) {
  // GitHub's tag endpoint excludes drafts; owner-visible release listing includes them.
  const release = JSON.parse(shell('gh', ['api', 'repos/sanirudh17/Typr/releases?per_page=100']))
    .find(release => release.tag_name === tag);
  assert.ok(release, 'Release not found in the repository.');
  assert.equal(release.tag_name, tag);
  assert.equal(release.body.trim().replaceAll('\r\n', '\n'), notes.replaceAll('\r\n', '\n'));
  assert.equal(release.assets.length, paths.length);
  for (const file of paths) {
    const asset = release.assets.find(asset => asset.name === path.basename(file));
    assert.ok(asset, `Asset not uploaded: ${file}`);
    assert.equal(asset.state, 'uploaded');
    assert.equal(asset.size, fs.statSync(file).size);
    assert.equal(asset.digest, `sha256:${digest(file)}`, `GitHub digest mismatch: ${file}`);
  }
  const remoteMain = shell('git', ['ls-remote', 'origin', 'refs/heads/main']).split(/\s/)[0];
  assert.equal(remoteMain, shell('git', ['rev-parse', 'HEAD']));
  if (process.argv.includes('--published')) {
    assert.equal(release.draft, false);
    assert.equal(release.prerelease, false);
    const latest = JSON.parse(shell('gh', ['api', 'repos/sanirudh17/Typr/releases/latest']));
    assert.equal(latest.tag_name, tag);
  }
  console.log(`GitHub release verified: ${release.html_url}; draft=${release.draft}; all uploaded digests match local files.`);
}
