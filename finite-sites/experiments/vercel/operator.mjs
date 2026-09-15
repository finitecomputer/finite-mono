// Disposable operator tooling, not a replacement release of fsite.
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import { parseEnv } from 'node:util';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { neon } from './control/node_modules/@neondatabase/serverless/index.mjs';

export const root = fileURLToPath(new URL('.', import.meta.url));
export const state = join(root, '.local-state');
export async function secrets() {
  return parseEnv(await readFile(join(state, 'operator.env'), 'utf8'));
}
export async function call(body, role = 'admin', endpoint) {
  const env = await secrets();
  const credential = role === 'issuer' ? env.POC_ISSUER_TOKEN : env.POC_ADMIN_TOKEN;
  const response = await fetch(endpoint ?? env.POC_CONTROL_URL, {
    method: 'POST', redirect: 'error', signal: AbortSignal.timeout(15000),
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${credential}` },
    body: JSON.stringify(body),
  });
  const result = await response.json();
  if (!response.ok) throw new Error(`Control API: ${response.status} ${result.error}`);
  return result;
}
export function vc(args, input, cwd) {
  const result = spawnSync('npm', ['exec', '--yes', '--package=vercel@59.17.0', '--', 'vercel', ...args], {
    encoding: 'utf8', input, cwd, maxBuffer: 4 * 1024 * 1024,
  });
  if (result.status !== 0) throw new Error(`Vercel command failed: ${result.stderr}`);
  return result.stdout;
}
async function main() {
  const [command, ...args] = process.argv.slice(2);
  if (command === 'init') {
    await mkdir(state, { recursive: true, mode: 0o700 });
    const envFile = join(state, 'operator.env');
    try { await readFile(envFile); }
    catch (error) {
      if (error.code !== 'ENOENT') throw error;
      await writeFile(envFile, [
        `POC_ADMIN_TOKEN=${randomBytes(32).toString('hex')}`,
        `POC_ISSUER_TOKEN=${randomBytes(32).toString('hex')}`,
        'POC_CONTROL_URL=https://finite-sites-poc-control.vercel.app/api/control',
      ].join('\n') + '\n', { mode: 0o600, flag: 'wx' });
    }
    const dbEnv = parseEnv(await readFile(join(root, 'control/.env.local'), 'utf8'));
    const sql = neon(dbEnv.DATABASE_URL);
    const statements = (await readFile(join(root, 'control/schema.sql'), 'utf8')).split(';').filter(s => s.trim());
    await sql.transaction(statements.map(statement => sql.query(statement)));
    console.log('Experiment schema initialized; operator secrets stored in ignored owner-only file.');
  } else if (command === 'push-control-env') {
    const env = await secrets();
    for (const key of ['POC_ADMIN_TOKEN', 'POC_ISSUER_TOKEN']) {
      for (const target of ['production', 'preview']) {
        vc(['env', 'add', key, target, '--force', '--cwd', join(root, 'control'), '--scope', 'alexlwn123-s-team'], env[key]);
      }
      console.log(`Configured ${key}`);
    }
  } else if (command === 'grant' || command === 'revoke') {
    console.log(await call({ op: 'grant.set', site: args[0], email: args[1], allowed: command === 'grant' }));
  } else if (command === 'disable' || command === 'enable') {
    console.log(await call({ op: 'site.disable', site: args[0], disabled: command === 'disable' }));
  } else {
    throw new Error('Usage: operator.mjs init | push-control-env | grant/revoke SITE EMAIL | disable/enable SITE');
  }
}
if (process.argv[1] === fileURLToPath(import.meta.url)) main().catch(error => { console.error(error.message); process.exitCode = 1; });
