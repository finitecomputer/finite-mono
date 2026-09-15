import { get, put } from '@vercel/blob';
import { hash, token, sameSecret, validToken, validSite, validEmail, siteHost, manifest, blobKey, sourceKey, requireThat } from './model.js';

// Synthetic operator/issuer boundary only. No browser cookie authorizes this API.
export async function control(sql, body, credential) {
  requireThat(body && validSite(body.site), 400, 'invalid_site');
  const site = body.site;
  // Managed Git publication has a per-Site credential; it cannot change shares,
  // register sites, or publish another project's content.
  const scopedOps=['site.status','version.begin','file.put','source.put','version.complete','version.activate'];
  const isAdmin=sameSecret(credential,process.env.POC_ADMIN_TOKEN);
  let scopedProject;
  if(!isAdmin&&body.op!=='viewer.issue'&&validToken(credential)&&scopedOps.includes(body.op)){
    [scopedProject]=await sql`SELECT p.* FROM poc2_projects p JOIN poc2_publishers a ON a.pubkey=p.owner_pubkey WHERE p.site_id=${site} AND p.publisher_hash=${hash(credential)} AND a.allowed`;
  }

  if (body.op === 'viewer.issue') {
    requireThat(sameSecret(credential, process.env.POC_ISSUER_TOKEN), 401, 'unauthorized');
    requireThat(validEmail(body.verified_email), 400, 'invalid_email');
    const email = body.verified_email.toLowerCase(), proof = token();
    const rows = await sql`INSERT INTO poc2_handoffs (token_hash,site_id,hostname,email,expires_at)
      SELECT ${hash(proof)},s.id,s.hostname,${email},now()+interval '60 seconds'
      FROM poc2_sites s JOIN poc2_grants g ON s.id=g.site_id
      WHERE s.id=${site} AND g.email=${email} AND NOT s.disabled RETURNING hostname`;
    requireThat(rows.length, 403, 'access_denied');
    return { proof, origin: `https://${rows[0].hostname}`, expiresIn: 60 };
  }
  requireThat(isAdmin||scopedProject,401,'unauthorized');
  if(body.op==='publisher.set'){
    requireThat(isAdmin&&validToken(body.pubkey)&&typeof body.allowed==='boolean',400,'invalid_publisher');
    await sql`INSERT INTO poc2_publishers(pubkey,allowed) VALUES(${body.pubkey},${body.allowed}) ON CONFLICT(pubkey) DO UPDATE SET allowed=EXCLUDED.allowed`;
    return {pubkey:body.pubkey,allowed:body.allowed};
  }
  if(body.op==='project.bind'){
    requireThat(isAdmin&&validToken(body.owner)&&validToken(body.publisherToken)&&/^https:\/\/github.com\/[a-zA-Z0-9-]+\/[a-zA-Z0-9_.-]+\.git$/.test(body.gitRemote)&&/^[a-zA-Z0-9_-]+$/.test(body.branch)&&/^[a-zA-Z0-9_/-]+$/.test(body.deployPath),400,'invalid_project');
    const rows=await sql`INSERT INTO poc2_projects(slug,site_id,owner_pubkey,git_remote_url,branch,deploy_path,publisher_hash)
      VALUES(${site},${site},${body.owner},${body.gitRemote},${body.branch},${body.deployPath},${hash(body.publisherToken)})
      ON CONFLICT(slug) DO UPDATE SET slug=EXCLUDED.slug WHERE poc2_projects.owner_pubkey=EXCLUDED.owner_pubkey AND poc2_projects.git_remote_url=EXCLUDED.git_remote_url AND poc2_projects.branch=EXCLUDED.branch AND poc2_projects.deploy_path=EXCLUDED.deploy_path AND poc2_projects.publisher_hash=EXCLUDED.publisher_hash RETURNING slug`;
    requireThat(rows.length,409,'conflicting_binding');return {site};
  }
  if (body.op === 'site.create') {
    const hostname = siteHost(site);
    requireThat(hostname, 400, 'hostname_not_configured');
    const rows = await sql`INSERT INTO poc2_sites (id,hostname) VALUES (${site},${hostname})
      ON CONFLICT (id) DO UPDATE SET id=EXCLUDED.id WHERE poc2_sites.hostname=EXCLUDED.hostname RETURNING id,hostname`;
    requireThat(rows.length, 409, 'conflicting_site');
    return { site, origin: `https://${hostname}` };
  }
  const sites = await sql`SELECT id,hostname,disabled,active_version FROM poc2_sites WHERE id=${site}`;
  requireThat(sites.length, 404, 'unknown_site');
  if (body.op === 'site.status') {
    const versions = await sql`SELECT id,source_commit,ready,created_at FROM poc2_versions WHERE site_id=${site} ORDER BY created_at DESC LIMIT 30`;
    return { ...sites[0], origin: `https://${sites[0].hostname}`, versions, deployment: process.env.VERCEL_URL || 'local' };
  }
  if (body.op === 'site.disable') {
    requireThat(typeof body.disabled === 'boolean', 400, 'invalid_disabled');
    await sql`UPDATE poc2_sites SET disabled=${body.disabled} WHERE id=${site}`;
    return { disabled: body.disabled };
  }
  if (body.op === 'grant.set') {
    requireThat(validEmail(body.email) && typeof body.allowed === 'boolean', 400, 'invalid_grant');
    const email = body.email.toLowerCase();
    if (body.allowed) await sql`INSERT INTO poc2_grants (site_id,email) VALUES (${site},${email}) ON CONFLICT DO NOTHING`;
    else await sql`DELETE FROM poc2_grants WHERE site_id=${site} AND email=${email}`;
    return { allowed: body.allowed };
  }
  if (body.op === 'version.begin') {
    const m = manifest(body);
    if(scopedProject)requireThat(m.deployPath===scopedProject.deploy_path,409,'deploy_path_mismatch');
    await sql.transaction([
      sql`INSERT INTO poc2_versions (site_id,id,source_commit,deploy_path) VALUES (${site},${m.version},${m.commit},${m.deployPath}) ON CONFLICT DO NOTHING`,
      sql`INSERT INTO poc2_files (site_id,version_id,path,sha256,size)
        SELECT ${site},${m.version},f.path,f.sha256,f.size FROM jsonb_to_recordset(${JSON.stringify(m.files)}::jsonb) AS f(path text,sha256 text,size integer)
        ON CONFLICT DO NOTHING`,
    ]);
    return { version: m.version };
  }
  requireThat(validToken(body.version), 400, 'invalid_version');
  if(body.op==='source.put'){
    requireThat(validToken(body.sha256)&&typeof body.base64==='string'&&body.base64.length<=2800000,400,'invalid_source');
    const bytes=Buffer.from(body.base64,'base64');
    requireThat(bytes.length<=2*1024*1024&&hash(bytes)===body.sha256&&bytes.toString('base64')===body.base64&&bytes.subarray(0,16).toString().startsWith('# v2 git bundle'),400,'invalid_source_bundle');
    const [v]=await sql`SELECT source_hash FROM poc2_versions WHERE site_id=${site} AND id=${body.version}`;
    requireThat(v&&(!v.source_hash||v.source_hash===body.sha256),409,'source_conflict');
    const pathname=sourceKey(site,body.sha256);
    try{await put(pathname,bytes,{access:'private',addRandomSuffix:false,allowOverwrite:false,contentType:'application/octet-stream'});}catch{}
    const stored=await get(pathname,{access:'private',useCache:false});
    requireThat(stored?.statusCode===200,503,'source_unavailable');
    requireThat(hash(Buffer.from(await new Response(stored.stream).arrayBuffer()))===body.sha256,503,'source_mismatch');
    const rows=await sql`UPDATE poc2_versions SET source_hash=${body.sha256} WHERE site_id=${site} AND id=${body.version} AND (source_hash IS NULL OR source_hash=${body.sha256}) RETURNING id`;
    requireThat(rows.length,409,'source_conflict');return {stored:true};
  }
  if (body.op === 'file.put') {
    const rows = await sql`SELECT path,sha256,size,uploaded FROM poc2_files WHERE site_id=${site} AND version_id=${body.version} AND path=${body.path}`;
    requireThat(rows.length, 404, 'unknown_file');
    const file = rows[0];
    requireThat(typeof body.base64 === 'string' && body.base64.length <= 1400000, 400, 'invalid_bytes');
    const bytes = Buffer.from(body.base64, 'base64');
    requireThat(bytes.toString('base64') === body.base64 && bytes.length === file.size && hash(bytes) === file.sha256, 400, 'content_mismatch');
    const pathname = blobKey(site, body.version, file);
    if (!file.uploaded) {
      // A put may have succeeded before a connection or DB write failed. Always
      // read back and verify; a matching immutable object makes that retry safe.
      let uploadFailure;
      try { await put(pathname, bytes, { access:'private', addRandomSuffix:false, allowOverwrite:false, contentType:'application/octet-stream' }); }
      catch (error) { uploadFailure=error; }
      const existing = await get(pathname, { access:'private', useCache:false });
      if (existing?.statusCode !== 200 && uploadFailure) throw uploadFailure;
      requireThat(existing?.statusCode === 200, 503, 'blob_unavailable');
      const stored = Buffer.from(await new Response(existing.stream).arrayBuffer());
      requireThat(stored.length === file.size && hash(stored) === file.sha256, 503, 'blob_mismatch');
      await sql`UPDATE poc2_files SET uploaded=true WHERE site_id=${site} AND version_id=${body.version} AND path=${file.path}`;
    }
    return { uploaded:true };
  }
  if (body.op === 'version.complete') {
    const rows = await sql`UPDATE poc2_versions v SET ready=true WHERE v.site_id=${site} AND v.id=${body.version}
      AND (v.source_hash IS NOT NULL OR NOT EXISTS(SELECT 1 FROM poc2_projects p WHERE p.site_id=v.site_id))
      AND EXISTS (SELECT 1 FROM poc2_files f WHERE f.site_id=v.site_id AND f.version_id=v.id)
      AND NOT EXISTS (SELECT 1 FROM poc2_files f WHERE f.site_id=v.site_id AND f.version_id=v.id AND NOT f.uploaded)
      RETURNING id`;
    requireThat(rows.length, 409, 'incomplete_version');
    return { version: body.version, ready:true };
  }
  if (body.op === 'version.activate') {
    requireThat(body.expectedVersion === null || validToken(body.expectedVersion), 400, 'expected_version_required');
    // Compare-and-swap prevents a stale publisher overwriting another publication.
    // Replaying a successful activation is safe while that version remains active.
    const rows = await sql`UPDATE poc2_sites s SET active_version=${body.version}
      WHERE s.id=${site} AND (s.active_version IS NOT DISTINCT FROM ${body.expectedVersion} OR s.active_version=${body.version})
      AND EXISTS (SELECT 1 FROM poc2_versions v WHERE v.site_id=s.id AND v.id=${body.version} AND v.ready) RETURNING active_version`;
    requireThat(rows.length, 409, 'activation_conflict_or_incomplete');
    return { activeVersion: body.version };
  }
  requireThat(false, 400, 'unknown_operation');
}
