import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { parseEnv } from 'node:util';
import { randomBytes, createHash } from 'node:crypto';
import { neon } from '@neondatabase/serverless';
import handler from './api/control.js';
import { secrets } from '../operator.mjs';

Object.assign(process.env, parseEnv(await readFile(new URL('.env.local', import.meta.url), 'utf8')), await secrets());
const sql = neon(process.env.DATABASE_URL);
const admin = process.env.POC_ADMIN_TOKEN;
const issuer = process.env.POC_ISSUER_TOKEN;
const gate = randomBytes(32).toString('hex');
const site = `test-${randomBytes(5).toString('hex')}`;
const other = `${site}-other`;
const email = 'viewer@example.invalid';
const hash = value => createHash('sha256').update(value).digest('hex');
async function request(body, credential = admin, method = 'POST') {
  const result = { headers: {} };
  await handler({ method, body: { site, ...body }, headers: { authorization: `Bearer ${credential}` } }, {
    setHeader(key, value) { result.headers[key] = value; },
    status(status) { result.status = status; return this; },
    json(body) { result.body = body; },
  });
  assert.match(result.headers['Cache-Control'], /no-store/);
  return result;
}
test('managed Postgres authority: replay, revocation, isolation and session lifetime', async t => {
  try {
    await t.test('admin required; create replay is safe; conflicting gate fails', async () => {
      assert.equal((await request({ op: 'site.create', gateToken: gate }, issuer)).status, 401);
      for (let i = 0; i < 2; i++) assert.equal((await request({ op: 'site.create', gateToken: gate })).status, 200);
      assert.equal((await request({ op: 'site.create', gateToken: randomBytes(32).toString('hex') })).status, 409);
      assert.equal((await request({ site: other, op: 'site.create', gateToken: randomBytes(32).toString('hex') })).status, 200);
    });
    await t.test('private default and credential isolation', async () => {
      assert.equal((await request({ op: 'gate.authorize' }, gate)).status, 403);
      assert.equal((await request({ site: other, op: 'gate.authorize' }, gate)).status, 401);
      assert.equal((await request({ op: 'viewer.issue', verified_email: email }, admin)).status, 401);
      assert.equal((await request({ op: 'viewer.issue', verified_email: email }, issuer)).status, 403);
      assert.equal((await request({ op: 'grant.set', email, allowed: true }, gate)).status, 401);
    });
    await t.test('grant is replay safe and bad inputs are rejected', async () => {
      for (let i = 0; i < 2; i++) assert.equal((await request({ op: 'grant.set', email, allowed: true })).status, 200);
      assert.equal((await request({ op: 'grant.set', email, allowed: 'true' })).status, 400);
      assert.equal((await request({ op: 'grant.set', email: 'not-email', allowed: true })).status, 400);
    });
    let session;
    await t.test('concurrent proof redemption has one winner', async () => {
      const issued = await request({ op: 'viewer.issue', verified_email: email }, issuer);
      assert.equal(issued.status, 200);
      const proof = issued.body.proof;
      const results = await Promise.all([request({ op: 'gate.redeem', proof }, gate), request({ op: 'gate.redeem', proof }, gate)]);
      assert.deepEqual(results.map(r => r.status).sort(), [200, 403]);
      session = results.find(r => r.status === 200).body.session;
      assert.equal((await request({ op: 'gate.authorize', session }, gate)).status, 200);
      assert.equal((await request({ op: 'gate.redeem', proof }, gate)).status, 403);
    });
    await t.test('revocation invalidates existing session and pending handoff', async () => {
      const issued = await request({ op: 'viewer.issue', verified_email: email }, issuer);
      for (let i = 0; i < 2; i++) assert.equal((await request({ op: 'grant.set', email, allowed: false })).status, 200);
      assert.equal((await request({ op: 'gate.authorize', session }, gate)).status, 403);
      assert.equal((await request({ op: 'gate.redeem', proof: issued.body.proof }, gate)).status, 403);
    });
    await t.test('expired proof cannot become session', async () => {
      await request({ op: 'grant.set', email, allowed: true });
      const issued = await request({ op: 'viewer.issue', verified_email: email }, issuer);
      await sql`UPDATE poc_handoffs SET expires_at=now()-interval '1 second' WHERE token_hash=${hash(issued.body.proof)}`;
      assert.equal((await request({ op: 'gate.redeem', proof: issued.body.proof }, gate)).status, 403);
    });
    await t.test('disabled site fails closed, including valid sessions', async () => {
      assert.equal((await request({ op: 'gate.authorize', session }, gate)).status, 200);
      for (let i = 0; i < 2; i++) assert.equal((await request({ op: 'site.disable', disabled: true })).status, 200);
      assert.equal((await request({ op: 'gate.authorize', session }, gate)).status, 403);
      assert.equal((await request({ op: 'viewer.issue', verified_email: email }, issuer)).status, 403);
      assert.equal((await request({ op: 'site.disable', disabled: 'false' })).status, 400);
      assert.equal((await request({ op: 'site.disable', disabled: false })).status, 200);
    });
    await t.test('expired session fails and credentials are never persisted raw', async () => {
      await sql`UPDATE poc_sessions SET expires_at=now()-interval '1 second' WHERE token_hash=${hash(session)}`;
      assert.equal((await request({ op: 'gate.authorize', session }, gate)).status, 403);
      const rows = await sql`SELECT gate_hash FROM poc_sites WHERE id=${site}`;
      assert.equal(rows[0].gate_hash, hash(gate));
    });
    await t.test('unavailable database fails closed', async () => {
      const saved = process.env.DATABASE_URL;
      process.env.DATABASE_URL = '';
      try { assert.equal((await request({ op: 'gate.authorize', session }, gate)).status, 503); }
      finally { process.env.DATABASE_URL = saved; }
    });
  } finally {
    await sql.transaction([
      sql`DELETE FROM poc_sessions WHERE site_id IN (${site},${other})`,
      sql`DELETE FROM poc_handoffs WHERE site_id IN (${site},${other})`,
      sql`DELETE FROM poc_grants WHERE site_id IN (${site},${other})`,
      sql`DELETE FROM poc_sites WHERE id IN (${site},${other})`,
    ]);
  }
});
