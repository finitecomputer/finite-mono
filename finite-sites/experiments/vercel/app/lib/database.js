import {neon} from '@neondatabase/serverless';
import {requireThat} from './model.js';
export function database(url=process.env.DATABASE_URL,schema=process.env.POC_DATABASE_SCHEMA){
 const base=neon(url);if(!schema)return base;
 requireThat(/^poc_restore_[a-f0-9]{16}$/.test(schema),500,'invalid_restore_schema');
 // A fresh restore uses transaction-local search_path. Never change pooled
 // connection state or interpolate a caller-controlled identifier.
 const execute=async queries=>(await base.transaction([
  base.query(`SET LOCAL search_path TO "${schema}"`),
  ...queries.map(q=>base.query(q.text,q.values)),
 ])).slice(1);
 const query=(text,values=[])=>({text,values,then(ok,fail){return execute([this]).then(rows=>rows[0]).then(ok,fail);}});
 const sql=(parts,...values)=>query(parts.reduce((s,p,i)=>s+(i?`$${i}`:'')+p,''),values);
 sql.query=query;sql.transaction=execute;return sql;
}
