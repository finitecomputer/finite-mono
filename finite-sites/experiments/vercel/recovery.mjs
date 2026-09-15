// Bounded one-Site empty-target drill. Recovery bytes live outside the provider.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {parseEnv} from 'node:util';
import {randomBytes} from 'node:crypto';
import {join} from 'node:path';
import {get,put} from './app/node_modules/@vercel/blob/dist/index.js';
import {database} from './app/lib/database.js';
import {hash,blobKey,sourceKey,requireThat} from './app/lib/model.js';
import {root,state} from './operator.mjs';
Object.assign(process.env,parseEnv(await readFile(join(root,'app/.env.local'),'utf8')));
const site='gamma',sql=database(),mode=process.argv[2]||'export';
const folder=join(state,'recovery');
const tables=['poc2_publishers','poc2_sites','poc2_versions','poc2_files','poc2_projects','poc2_native_grants'];
if(mode==='export'){
 await mkdir(join(folder,'objects'),{recursive:true,mode:0o700});
 // One relational snapshot; immutable objects can be downloaded afterward.
 const rows=await sql.transaction([
  sql`SELECT * FROM poc2_publishers WHERE pubkey IN (SELECT owner_pubkey FROM poc2_projects WHERE site_id=${site})`,
  sql`SELECT * FROM poc2_sites WHERE id=${site}`,
  sql`SELECT * FROM poc2_versions WHERE site_id=${site} AND ready ORDER BY version_no LIMIT 21`,
  sql`SELECT f.* FROM poc2_files f JOIN poc2_versions v ON f.site_id=v.site_id AND f.version_id=v.id WHERE f.site_id=${site} AND v.ready ORDER BY version_id,path LIMIT 4001`,
  sql`SELECT * FROM poc2_projects WHERE site_id=${site}`,
  sql`SELECT * FROM poc2_native_grants WHERE site_id=${site} ORDER BY pubkey LIMIT 101`,
 ],{isolationLevel:'RepeatableRead',readOnly:true});
 requireThat(rows[1].length===1&&rows[2].length>0&&rows[2].length<=20&&rows[3].length<=4000&&rows[5].length<=100,400,'snapshot_bounds');
 const objects=new Map();
 for(const file of rows[3])objects.set(blobKey(site,file.version_id,file),file.sha256);
 for(const version of rows[2]){requireThat(version.source_hash,400,'missing_source');objects.set(sourceKey(site,version.source_hash),version.source_hash);}
 const manifest={format:'finite-sites-poc-recovery-v1',site,capturedAt:new Date().toISOString(),tables:Object.fromEntries(tables.map((name,i)=>[name,rows[i]])),objects:[]};
 for(const [key,digest] of objects){
  const object=await get(key,{access:'private',useCache:false});requireThat(object?.statusCode===200,503,'snapshot_object_missing');
  const bytes=Buffer.from(await new Response(object.stream).arrayBuffer());requireThat(hash(bytes)===digest,500,'snapshot_hash_mismatch');
  await writeFile(join(folder,'objects',digest),bytes,{mode:0o600});manifest.objects.push({key,sha256:digest,size:bytes.length});
 }
 const bytes=JSON.stringify(manifest,null,2);await writeFile(join(folder,'manifest.json'),bytes,{mode:0o600});await writeFile(join(folder,'manifest.sha256'),hash(bytes)+'\n',{mode:0o600});
 console.log(JSON.stringify({exported:true,site,versions:rows[2].length,objects:objects.size,snapshot:folder}));
}else if(mode==='restore'){
 const raw=await readFile(join(folder,'manifest.json'));requireThat(hash(raw)===(await readFile(join(folder,'manifest.sha256'),'utf8')).trim(),400,'snapshot_checksum_mismatch');
 const manifest=JSON.parse(raw);requireThat(manifest.format==='finite-sites-poc-recovery-v1'&&manifest.site===site&&manifest.objects.length<=4020,400,'invalid_snapshot');
 // Verify every object before creating or mutating the target.
 for(const object of manifest.objects){requireThat(/^[a-f0-9]{64}$/.test(object.sha256)&&object.key.startsWith(`sites/${site}/`)||/^[a-f0-9]{64}$/.test(object.sha256)&&object.key.startsWith(`sources/${site}/`),400,'invalid_object');const bytes=await readFile(join(folder,'objects',object.sha256));requireThat(bytes.length===object.size&&hash(bytes)===object.sha256,400,'corrupt_recovery_object');}
 const suffix=randomBytes(8).toString('hex'),schema=`poc_restore_${suffix}`,prefix=`restore-${suffix}`;
 await sql.query(`CREATE SCHEMA "${schema}"`);const target=database(process.env.DATABASE_URL,schema);
 for(const filename of ['schema.sql','fidelity-schema.sql']){
  const statements=(await readFile(join(root,'app',filename),'utf8')).split(/;\n(?=CREATE|ALTER|DO|$)/).filter(s=>s.trim());
  await target.transaction(statements.map(s=>target.query(s)));
 }
 for(const object of manifest.objects){const bytes=await readFile(join(folder,'objects',object.sha256));await put(`${prefix}/${object.key}`,bytes,{access:'private',addRandomSuffix:false,allowOverwrite:false,contentType:'application/octet-stream'});const verify=await get(`${prefix}/${object.key}`,{access:'private',useCache:false});requireThat(verify?.statusCode===200&&hash(Buffer.from(await new Response(verify.stream).arrayBuffer()))===object.sha256,503,'restore_readback_mismatch');}
 const insertions=[];
 for(const table of tables){
  for(const original of manifest.tables[table]){
   const row={...original};if(table==='poc2_sites')row.active_version=null;
   // Recordset uses the target's schema-defined row type, never snapshot column identifiers.
   insertions.push(target.query(`INSERT INTO ${table} SELECT * FROM json_populate_record(NULL::${table},$1::json)`,[JSON.stringify(row)]));
  }
 }
 const active=manifest.tables.poc2_sites[0].active_version;
 insertions.push(target`UPDATE poc2_sites SET active_version=${active} WHERE id=${site}`);
 insertions.push(target.query("SELECT setval(pg_get_serial_sequence('poc2_versions','version_no'),(SELECT max(version_no) FROM poc2_versions))"));
 await target.transaction(insertions);
 const restore={schema,blobPrefix:prefix,site,hostname:manifest.tables.poc2_sites[0].hostname,activeVersion:active,versions:manifest.tables.poc2_versions.length,objects:manifest.objects.length,restoredAt:new Date().toISOString()};
 await writeFile(join(state,'restored.json'),JSON.stringify(restore,null,2));
 console.log(JSON.stringify(restore));
}else throw new Error('Usage: recovery.mjs export | restore');
