import { neon } from '@neondatabase/serverless';
import { createHash, randomBytes, timingSafeEqual } from 'node:crypto';

const hash = value => createHash('sha256').update(value).digest('hex');
const token = () => randomBytes(32).toString('hex');
const validToken = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
const sameSecret = (a, b) => validToken(a) && validToken(b) && timingSafeEqual(Buffer.from(a), Buffer.from(b));
const validSite = value => typeof value === 'string' && /^[a-z][a-z0-9-]{2,62}$/.test(value);
const validEmail = value => typeof value === 'string' && value.length <= 254 && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value);

// Each operation authenticates its own capability. No client-supplied identity
// header grants access. Only the test issuer may assert a verified email.
export default async function handler(req, res) {
  res.setHeader('Cache-Control', 'private, no-store');
  res.setHeader('CDN-Cache-Control', 'no-store');
  res.setHeader('Vercel-CDN-Cache-Control', 'no-store');
  res.setHeader('X-Content-Type-Options', 'nosniff');
  const send = (status, body) => res.status(status).json(body);
  if (req.method === 'GET') return send(200, { experiment: 'finite-sites-vercel', ok: true });
  if (req.method !== 'POST') return send(405, { error: 'method_not_allowed' });
  try {
    let body = req.body;
    if (typeof body === 'string' || Buffer.isBuffer(body)) {
      if (Buffer.byteLength(body) > 4096) return send(413, { error: 'body_too_large' });
      body = JSON.parse(body.toString());
    }
    if (!body || Buffer.byteLength(JSON.stringify(body)) > 4096 || !validSite(body.site)) return send(400, { error: 'invalid_request' });
    const credential = (req.headers.authorization ?? '').replace(/^Bearer /, '');
    const sql = neon(process.env.DATABASE_URL);
    if (['site.create', 'site.disable', 'grant.set'].includes(body.op)) {
      if (!sameSecret(credential, process.env.POC_ADMIN_TOKEN)) return send(401, { error: 'unauthorized' });
      if (body.op === 'site.create') {
        if (!validToken(body.gateToken)) return send(400, { error: 'invalid_gate_token' });
        const gateHash = hash(body.gateToken);
        const rows = await sql`INSERT INTO poc_sites (id, gate_hash) VALUES (${body.site}, ${gateHash})
          ON CONFLICT (id) DO UPDATE SET id = EXCLUDED.id
          WHERE poc_sites.gate_hash = EXCLUDED.gate_hash RETURNING id`;
        return rows.length ? send(200, { site: body.site }) : send(409, { error: 'conflicting_site' });
      }
      if (body.op === 'site.disable') {
        if (typeof body.disabled !== 'boolean') return send(400, { error: 'invalid_disabled' });
        const rows = await sql`UPDATE poc_sites SET disabled = ${body.disabled} WHERE id = ${body.site} RETURNING id`;
        return rows.length ? send(200, { disabled: body.disabled }) : send(404, { error: 'unknown_site' });
      }
      if (!validEmail(body.email) || typeof body.allowed !== 'boolean') return send(400, { error: 'invalid_grant' });
      const email = body.email.toLowerCase();
      if (!(await sql`SELECT id FROM poc_sites WHERE id = ${body.site}`).length) return send(404, { error: 'unknown_site' });
      if (body.allowed) await sql`INSERT INTO poc_grants (site_id,email) VALUES (${body.site},${email}) ON CONFLICT DO NOTHING`;
      else await sql`DELETE FROM poc_grants WHERE site_id = ${body.site} AND email = ${email}`;
      return send(200, { allowed: body.allowed });
    }
    if (body.op === 'viewer.issue') {
      if (!sameSecret(credential, process.env.POC_ISSUER_TOKEN)) return send(401, { error: 'unauthorized' });
      if (!validEmail(body.verified_email)) return send(400, { error: 'invalid_email' });
      const email = body.verified_email.toLowerCase();
      const proof = token();
      const rows = await sql`INSERT INTO poc_handoffs (token_hash,site_id,email,expires_at)
        SELECT ${hash(proof)}, s.id, ${email}, now() + interval '60 seconds'
        FROM poc_sites s JOIN poc_grants g ON g.site_id=s.id
        WHERE s.id=${body.site} AND g.email=${email} AND NOT s.disabled RETURNING site_id`;
      return rows.length ? send(200, { proof, expiresIn: 60 }) : send(403, { error: 'access_denied' });
    }
    if (!['gate.authorize', 'gate.redeem'].includes(body.op) || !validToken(credential)) return send(401, { error: 'unauthorized' });
    const sites = await sql`SELECT id,visibility,disabled FROM poc_sites WHERE id=${body.site} AND gate_hash=${hash(credential)}`;
    if (!sites.length) return send(401, { error: 'unauthorized' });
    if (sites[0].disabled) return send(403, { error: 'access_denied' });
    if (body.op === 'gate.redeem') {
      if (!validToken(body.proof)) return send(400, { error: 'invalid_proof' });
      const session = token();
      // One transaction consumes the proof and creates the session, rechecking
      // the current grant. Concurrent redemption has exactly one winner.
      const rows = await sql`WITH consumed AS (
        UPDATE poc_handoffs h SET consumed_at=now()
        WHERE h.token_hash=${hash(body.proof)} AND h.site_id=${body.site}
          AND h.expires_at>now() AND h.consumed_at IS NULL
          AND EXISTS (SELECT 1 FROM poc_grants g WHERE g.site_id=h.site_id AND g.email=h.email)
        RETURNING h.site_id,h.email
      ) INSERT INTO poc_sessions (token_hash,site_id,email,expires_at)
        SELECT ${hash(session)},site_id,email,now()+interval '1 hour' FROM consumed RETURNING site_id`;
      return rows.length ? send(200, { session, maxAge: 3600 }) : send(403, { error: 'invalid_or_used_proof' });
    }
    if (sites[0].visibility === 'public') return send(200, { allowed: true });
    if (!validToken(body.session)) return send(403, { error: 'access_denied' });
    // Cookies identify the viewer; permissions are never cached in the cookie.
    const rows = await sql`SELECT s.site_id FROM poc_sessions s JOIN poc_grants g
      ON s.site_id=g.site_id AND s.email=g.email
      WHERE s.token_hash=${hash(body.session)} AND s.site_id=${body.site} AND s.expires_at>now()`;
    return rows.length ? send(200, { allowed: true }) : send(403, { error: 'access_denied' });
  } catch {
    return send(503, { error: 'authorization_unavailable' });
  }
}
