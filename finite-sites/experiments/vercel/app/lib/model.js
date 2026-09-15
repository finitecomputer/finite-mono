import { createHash, randomBytes, timingSafeEqual } from 'node:crypto';

// Explicit disposable-experiment limits keep uploads below Vercel's body limit.
export const MAX_FILES = 200, MAX_FILE_BYTES = 1024 * 1024, MAX_VERSION_BYTES = 8 * 1024 * 1024;
export const COOKIE = '__Host-finite_poc';
export const hash = value => createHash('sha256').update(value).digest('hex');
export const token = () => randomBytes(32).toString('hex');
export const validToken = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
export const sameSecret = (a, b) => validToken(a) && validToken(b) && timingSafeEqual(Buffer.from(a), Buffer.from(b));
export const validSite = value => typeof value === 'string' && /^[a-z][a-z0-9-]{1,61}[a-z0-9]$/.test(value) && !['www','api','auth','admin','control'].includes(value);
export const validEmail = value => typeof value === 'string' && value.length <= 254 && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value);
export const validPath = value => typeof value === 'string' && value.length <= 512 && /^[a-zA-Z0-9_./@+ -]+$/.test(value) && !value.startsWith('_finite') && value.split('/').every(p => p && p !== '..' && (!p.startsWith('.') || p === '.well-known'));
export function fault(status, error) { return Object.assign(new Error(error), { status }); }
export function requireThat(ok, status, error) { if (!ok) throw fault(status, error); }
export function siteHost(site, env = process.env) {
  requireThat(validSite(site), 400, 'invalid_site');
  return env.POC_SITE_BASE_DOMAIN ? `${site}.${env.POC_SITE_BASE_DOMAIN}` : null;
}
export function resolveHost(raw, env = process.env) {
  // Trust only the real request Host. Forwarded/tenant headers are never authority.
  if (typeof raw !== 'string' || !/^[a-zA-Z0-9.-]+$/.test(raw)) return null;
  const host = raw.toLowerCase();
  if (host === env.POC_CONTROL_HOST) return { control: true, host };
  const suffix = env.POC_SITE_BASE_DOMAIN;
  if (suffix && host === `admin.${suffix}`) return { admin: true, host };
  if (suffix && host.endsWith(`.${suffix}`)) {
    const site = host.slice(0, -suffix.length - 1);
    if (validSite(site)) return { site, host };
  }
  return null;
}
export function manifest(input) {
  requireThat(input && /^[a-f0-9]{40}$/.test(input.commit) && typeof input.deployPath === 'string' && /^[a-zA-Z0-9_/-]+$/.test(input.deployPath), 400, 'invalid_source');
  requireThat(Array.isArray(input.files) && input.files.length > 0 && input.files.length <= MAX_FILES, 400, 'invalid_manifest');
  let total = 0;
  const paths = new Set();
  const files = input.files.map(f => {
    requireThat(f && validPath(f.path) && validToken(f.sha256) && Number.isSafeInteger(f.size) && f.size >= 0 && f.size <= MAX_FILE_BYTES && !paths.has(f.path), 400, 'invalid_file');
    paths.add(f.path); total += f.size;
    return { path: f.path, sha256: f.sha256, size: f.size };
  }).sort((a,b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  requireThat(paths.has('index.html') && total <= MAX_VERSION_BYTES, 400, 'invalid_deploy_tree');
  const value = { commit: input.commit, deployPath: input.deployPath, files };
  return { ...value, version: hash(JSON.stringify(value)) };
}
export const objectKey = key => process.env.POC_BLOB_PREFIX ? `${process.env.POC_BLOB_PREFIX}/${key}` : key;
export const sourceKey = (site,digest) => objectKey(`sources/${site}/${digest}.bundle`);
export const blobKey = (site, version, file) => objectKey(`sites/${site}/versions/${version}/${file.sha256}`);
export function contentType(path) {
  const extension = path.split('.').at(-1).toLowerCase();
  return ({ html:'text/html; charset=utf-8', css:'text/css; charset=utf-8', js:'text/javascript; charset=utf-8', mjs:'text/javascript; charset=utf-8', txt:'text/plain; charset=utf-8', json:'application/json', svg:'image/svg+xml', png:'image/png', jpg:'image/jpeg', jpeg:'image/jpeg', ico:'image/x-icon', pdf:'application/pdf', woff2:'font/woff2', webp:'image/webp' })[extension] || 'application/octet-stream';
}
