import { schnorr } from '@noble/curves/secp256k1.js';
import { bech32 } from '@scure/base';
import { hash,token,validToken,requireThat,sameSecret } from './model.js';
export const npub=hex=>bech32.encode('npub',bech32.toWords(Buffer.from(hex,'hex')));
export function pubkey(value){try{const d=bech32.decode(value);const hex=Buffer.from(bech32.fromWords(d.words)).toString('hex');requireThat(d.prefix==='npub'&&validToken(hex),400,'invalid_npub');return hex;}catch{requireThat(false,400,'invalid_npub');}}
export function verifyNative(authorization,url,method,body,now=Math.floor(Date.now()/1000)){
 requireThat(typeof authorization==='string'&&authorization.length<=8192&&authorization.startsWith('Nostr '),401,'native_auth_required');
 let e;try{e=JSON.parse(Buffer.from(authorization.slice(6),'base64').toString());}catch{requireThat(false,401,'invalid_event');}
 requireThat(e&&e.kind===27235&&e.content===''&&Number.isSafeInteger(e.created_at)&&Math.abs(now-e.created_at)<=60&&validToken(e.pubkey)&&validToken(e.id)&&typeof e.sig==='string'&&/^[a-f0-9]{128}$/.test(e.sig)&&Array.isArray(e.tags)&&e.tags.length<=8,401,'invalid_event');
 const tags=new Map();
 for(const tag of e.tags){requireThat(Array.isArray(tag)&&tag.length===2&&tag.every(v=>typeof v==='string')&&!tags.has(tag[0]),401,'invalid_tags');tags.set(tag[0],tag[1]);}
 requireThat(tags.get('u')===url&&tags.get('method')===method,401,'request_mismatch');
 requireThat(body===null?!tags.has('payload'):(tags.get('payload')===hash(body)||(body.length===0&&!tags.has('payload'))),401,'payload_mismatch');
 const id=hash(JSON.stringify([0,e.pubkey,e.created_at,e.kind,e.tags,e.content]));
 requireThat(id===e.id&&schnorr.verify(Buffer.from(e.sig,'hex'),Buffer.from(id,'hex'),Buffer.from(e.pubkey,'hex')),401,'invalid_signature');
 return e.pubkey;
}
export function viewerRequest(raw){
 let body;try{body=JSON.parse(raw);}catch{requireThat(false,400,'invalid_request');}
 requireThat(Buffer.byteLength(raw)<=4096&&Object.keys(body).sort().join(',')==='client,nonce,purpose,return_to'&&body.purpose==='finite_site_view_session'&&typeof body.return_to==='string'&&body.return_to.length<=1024&&/^\/(?!\/)[\x21-\x7e]*$/.test(body.return_to)&&!body.return_to.includes('\\')&&typeof body.client==='string'&&/^[a-zA-Z0-9_-]{1,64}$/.test(body.client)&&typeof body.nonce==='string'&&/^[a-zA-Z0-9_-]{16,128}$/.test(body.nonce),400,'invalid_viewer_request');
 return body;
}
export async function issueNative(sql,site,authorization,raw){
 const body=viewerRequest(raw),who=verifyNative(authorization,`https://${site.hostname}/_finite/auth/native-session`,'POST',raw),proof=token();
 const rows=await sql`INSERT INTO poc2_native_handoffs (token_hash,site_id,pubkey,hostname,return_to,nonce,expires_at)
 SELECT ${hash(proof)},s.id,${who},s.hostname,${body.return_to},${body.nonce},now()+interval '60 seconds'
 FROM poc2_sites s WHERE s.id=${site.id} AND NOT s.disabled AND EXISTS (SELECT 1 FROM poc2_native_grants g WHERE g.site_id=s.id AND g.pubkey=${who})
 ON CONFLICT (site_id,pubkey,nonce) DO NOTHING RETURNING site_id`;
 requireThat(rows.length,403,'access_denied_or_replay');
 return {redeem_url:`https://${site.hostname}/_finite/auth?native_token=${proof}&return_to=${encodeURIComponent(body.return_to)}`};
}
export async function redeemNative(sql,site,proof,returnTo){
 requireThat(validToken(proof),403,'invalid_proof');const session=token();
 const rows=await sql`WITH consumed AS (UPDATE poc2_native_handoffs h SET consumed_at=now()
 WHERE h.token_hash=${hash(proof)} AND h.site_id=${site.id} AND h.hostname=${site.hostname} AND h.return_to=${returnTo} AND h.expires_at>now() AND h.consumed_at IS NULL
 AND EXISTS (SELECT 1 FROM poc2_native_grants g WHERE g.site_id=h.site_id AND g.pubkey=h.pubkey)
 RETURNING h.site_id,h.pubkey,h.hostname,h.return_to)
 INSERT INTO poc2_native_sessions (token_hash,site_id,pubkey,hostname,expires_at)
 SELECT ${hash(session)},site_id,pubkey,hostname,now()+interval '1 hour' FROM consumed RETURNING site_id`;
 requireThat(rows.length,403,'access_denied');return session;
}
export async function exchangeNative(sql,raw,credential){
 requireThat(sameSecret(credential,process.env.POC_ISSUER_TOKEN),401,'unauthorized');
 const value=JSON.parse(raw);requireThat(typeof value.signed_body==='string'&&value.signed_body.length<=4096,400,'invalid_request');
 const origin=value.site_url||value.output_url;
 const rows=await sql`SELECT id,hostname FROM poc2_sites WHERE 'https://' || hostname || '/'=${origin}`;
 requireThat(rows.length,403,'unknown_site');return issueNative(sql,rows[0],value.authorization,value.signed_body);
}
export async function nativeApi(sql,path,method,raw,authorization,origin){
 const who=verifyNative(authorization,origin+path,method,method==='GET'?null:raw);
 if(path==='/api/v2/auth/register'&&method==='POST'){
  requireThat(!raw,400,'body_not_allowed');
  const rows=await sql`SELECT pubkey FROM poc2_publishers WHERE pubkey=${who} AND allowed`;
  requireThat(rows.length,403,'publisher_not_admitted');
  return {pubkey:who,npub:npub(who),principal_id:who,grant_source:'prototype-admission',registered:true,site_limit:10};
 }
 const admitted=await sql`SELECT pubkey FROM poc2_publishers WHERE pubkey=${who} AND allowed`;
 requireThat(admitted.length,403,'publisher_not_admitted');
 if(path==='/api/v2/projects/init'&&method==='POST'){
  const request=JSON.parse(raw),cfg=request.config;
  requireThat(!request.owner_email&&!request.hosted_requester_assertion&&!request.requesting_user_npub,400,'account_ownership_not_supported_in_prototype');
  const [p]=await sql`SELECT p.*,s.hostname,s.active_version FROM poc2_projects p JOIN poc2_sites s ON s.id=p.site_id WHERE p.slug=${cfg?.project?.slug} AND p.owner_pubkey=${who}`;
  requireThat(p,403,'project_not_bound_to_publisher');
  requireThat(cfg.site&&(cfg.site.name||cfg.project.slug)===p.site_id&&cfg.site.branch===p.branch&&cfg.site.path===p.deploy_path&&!cfg.site.spa&&!Object.keys(cfg.outputs||{}).length,409,'configuration_mismatch');
  return {dry_run:!!request.dry_run,created:false,project_id:p.slug,slug:p.slug,project_visibility:'private',git_remote_url:p.git_remote_url,finite_toml:`[project]\nslug = "${p.slug}"\n\n[site]\nname = "${p.site_id}"\nbranch = "${p.branch}"\npath = "${p.deploy_path}"\n`,site:await summary(sql,p)};
 }
 const match=/^\/api\/v2\/projects\/([a-z][a-z0-9-]+)(\/site\/sharing)?$/.exec(path);
 requireThat(match,404,'unsupported_prototype_route');
 const [p]=await sql`SELECT p.*,s.hostname,s.active_version FROM poc2_projects p JOIN poc2_sites s ON s.id=p.site_id WHERE p.slug=${match[1]} AND p.owner_pubkey=${who}`;
 requireThat(p,403,'not_project_owner');
 if(method==='GET'&&!match[2])return {project_id:p.slug,slug:p.slug,role:'owner',project_visibility:'private',git_remote_url:p.git_remote_url,site:await summary(sql,p),collaborators:[]};
 requireThat(method==='POST'&&match[2],405,'method_not_allowed');
 const body=JSON.parse(raw);
 requireThat(!body.visibility||body.visibility==='private',400,'prototype_private_only');
 requireThat(!(body.add_emails?.length||body.remove_emails?.length||body.send_invite),400,'native_shares_only');
 const add=body.add_npubs||[],remove=body.remove_npubs||[];
 requireThat(Array.isArray(add)&&Array.isArray(remove)&&add.length+remove.length<=100,400,'too_many_grants');
 const adds=add.map(pubkey),removes=remove.map(pubkey);
 requireThat(!adds.some(k=>removes.includes(k)),400,'conflicting_grants');
 if(adds.length||removes.length)await sql.transaction([
  ...adds.map(key=>sql`INSERT INTO poc2_native_grants (site_id,pubkey) VALUES (${p.site_id},${key}) ON CONFLICT DO NOTHING`),
  ...removes.map(key=>sql`DELETE FROM poc2_native_grants WHERE site_id=${p.site_id} AND pubkey=${key}`),
 ]);
 const grants=await sql`SELECT pubkey FROM poc2_native_grants WHERE site_id=${p.site_id} ORDER BY pubkey`;
 return {project_slug:p.slug,site_name:p.site_id,site_url:`https://${p.hostname}/`,visibility:'private',shared_emails:[],shared_npubs:grants.map(g=>npub(g.pubkey)),invited_emails:[]};
}
async function summary(sql,p){
 const rows=p.active_version?await sql`SELECT version_no FROM poc2_versions WHERE site_id=${p.site_id} AND id=${p.active_version}`:[];
 return {name:p.site_id,url:`https://${p.hostname}/`,site_id:p.site_id,status:p.active_version?'published':'draft',visibility:'private',active_version:rows[0]?.version_no||null,branch:p.branch,path:p.deploy_path,spa:false,created:false};
}
