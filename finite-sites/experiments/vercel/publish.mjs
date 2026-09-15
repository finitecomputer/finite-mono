import { execFileSync } from 'node:child_process';
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createHash, randomBytes } from 'node:crypto';
import { join, resolve } from 'node:path';
import { call, root, secrets, state, vc } from './operator.mjs';

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
  if (!['100644', '100755'].includes(mode) || kind !== 'blob' || !relative || relative.startsWith('_finite/') ||
      relative.split('/').some(p => p === '..' || (p.startsWith('.') && p !== '.well-known')) || /[<>"\\\x00-\x1f]/.test(relative)) {
    throw new Error(`Unsupported deploy entry: ${path}`);
  }
  const bytes = git(['cat-file', 'blob', oid]);
  total += bytes.length;
  if (total > 8 * 1024 * 1024) throw new Error('Experiment limit: 8 MiB per site');
  files.push({ path: relative, bytes, sha256: createHash('sha256').update(bytes).digest('hex') });
}
if (!files.some(f => f.path === 'index.html')) throw new Error('Deploy tree needs index.html');
const directory = join(state, 'sites', site);
await mkdir(directory, { recursive: true, mode: 0o700 });
let gateToken;
try { gateToken = (await readFile(join(directory, 'gate-token'), 'utf8')).trim(); }
catch (error) {
  if (error.code !== 'ENOENT') throw error;
  gateToken = randomBytes(32).toString('hex');
  await writeFile(join(directory, 'gate-token'), gateToken, { mode: 0o600, flag: 'wx' });
}
await call({ op: 'site.create', site, gateToken });
for (const file of ['middleware.js', 'vercel.json', 'package.json', 'package-lock.json']) {
  await cp(join(root, 'site-template', file), join(directory, file));
}
// Explicit upload allowlist prevents operator credentials entering deployments.
await writeFile(join(directory, '.vercelignore'), '*\n!public\n!public/**\n!middleware.js\n!package.json\n!package-lock.json\n!vercel.json\n');
await rm(join(directory, 'public'), { recursive: true, force: true });
for (const file of files) {
  const target = join(directory, 'public', file.path);
  await mkdir(join(target, '..'), { recursive: true });
  await writeFile(target, file.bytes);
}
const manifest = { site, commit, deployPath, files: files.map(({path,sha256}) => ({path,sha256})) };
await writeFile(join(directory, 'manifest.json'), JSON.stringify(manifest, null, 2));
const project = `finite-sites-poc-${site}`;
try { await readFile(join(directory, '.vercel/project.json')); }
catch (error) {
  if (error.code !== 'ENOENT') throw error;
  vc(['link', '--yes', '--project', project, '--cwd', directory, '--scope', 'alexlwn123-s-team']);
}
const env = await secrets();
for (const [key, value] of Object.entries({ POC_SITE_ID: site, POC_GATE_TOKEN: gateToken, POC_CONTROL_URL: env.POC_CONTROL_URL })) {
  for (const target of ['production', 'preview']) {
    vc(['env', 'add', key, target, '--force', '--cwd', directory, '--scope', 'alexlwn123-s-team'], value);
  }
}
const output = vc(['deploy', '--yes', '--prod', '--cwd', directory, '--scope', 'alexlwn123-s-team']);
let deployment;
try { deployment = JSON.parse(output).deployment.url; }
catch { deployment = output.trim().split('\n').findLast(line => /^https:\/\//.test(line)); }
if (!deployment) throw new Error('Deployment finished but no URL was returned; inspect the Vercel project before retrying');
const receipt = { ...manifest, project, deployment, origin: `https://${project}.vercel.app`, publishedAt: new Date().toISOString() };
await writeFile(join(directory, `receipt-${commit}.json`), JSON.stringify(receipt, null, 2));
console.log(JSON.stringify(receipt, null, 2));
