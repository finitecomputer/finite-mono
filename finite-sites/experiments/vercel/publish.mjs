import { execFileSync } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join, resolve } from 'node:path';
import { call, state } from './operator.mjs';
import { validPath } from './app/lib/model.js';

// Select committed bytes only. Never run a customer's package scripts or copy
// their middleware, server code, configuration, symlinks or untracked files.
const [site, repository, revision, deployPath = 'site'] = process.argv.slice(2);
if (!/^[a-z][a-z0-9-]{2,62}$/.test(site ?? '') || !repository || !revision || !/^[a-zA-Z0-9_/-]+$/.test(deployPath) || deployPath.split('/').some(p => !p || p === '..')) {
  throw new Error('Usage: publish.mjs SITE REPOSITORY COMMIT [DEPLOY_PATH]');
}
const git = args => execFileSync('git', ['-C', resolve(repository), ...args], { maxBuffer: 16 * 1024 * 1024 });
const commit = git(['rev-parse', '--verify', '--end-of-options', `${revision}^{commit}`]).toString().trim();
const listing = git(['ls-tree', '-rz', '--full-tree', commit, '--', deployPath]).toString().split('\0').filter(Boolean);
if (!listing.length || listing.length > 200) throw new Error('Experiment limit: 1–200 committed files');
const files = [];
let total = 0;
for (const entry of listing) {
  const separator = entry.indexOf('\t');
  const metadata = entry.slice(0, separator);
  const path = entry.slice(separator + 1);
  const [mode, kind, oid] = metadata.split(' ');
  const relative = path.slice(deployPath.length + 1);
  if (!['100644', '100755'].includes(mode) || kind !== 'blob' || !validPath(relative) ||
      relative.split('/').some(p => p === '..' || (p.startsWith('.') && p !== '.well-known')) || /[<>"\\\x00-\x1f]/.test(relative)) {
    throw new Error(`Unsupported deploy entry: ${path}`);
  }
  const bytes = git(['cat-file', 'blob', oid]);
  total += bytes.length;
  if (total > 8 * 1024 * 1024) throw new Error('Experiment limit: 8 MiB per site');
  if (bytes.length > 1024 * 1024) throw new Error('Experiment limit: 1 MiB per file');
  files.push({ path: relative, bytes, size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') });
}
if (!files.some(f => f.path === 'index.html')) throw new Error('Deploy tree needs index.html');
const { origin } = await call({ op: 'site.create', site });
const before = await call({ op: 'site.status', site });
const manifest = { site, commit, deployPath, files: files.map(({path,sha256,size}) => ({path,sha256,size})) };
const { version } = await call({ op: 'version.begin', ...manifest });
for (const file of files) {
  await call({ op: 'file.put', site, version, path: file.path, base64: file.bytes.toString('base64') });
}
await call({ op: 'version.complete', site, version });
await call({ op: 'version.activate', site, version, expectedVersion: before.active_version });
const receipt = { ...manifest, version, origin, platformDeployment: before.deployment, publishedAt: new Date().toISOString() };
const directory = join(state, 'sites', site);
await mkdir(directory, { recursive: true, mode: 0o700 });
await writeFile(join(directory, `receipt-${commit}.json`), JSON.stringify(receipt, null, 2));
console.log(JSON.stringify(receipt, null, 2));
