import {readFile} from 'node:fs/promises';
import {parseEnv} from 'node:util';
import {createServer} from 'node:http';
import {root,state} from './operator.mjs';
const restored=JSON.parse(await readFile(`${state}/restored.json`));
const config=JSON.parse(await readFile(`${root}/config.json`));
Object.assign(process.env,parseEnv(await readFile(`${root}/app/.env.local`,'utf8')),{POC_DATABASE_SCHEMA:restored.schema,POC_BLOB_PREFIX:restored.blobPrefix,POC_CONTROL_HOST:config.controlHost,POC_SITE_BASE_DOMAIN:config.siteBaseDomain});
const {default:handler}=await import('./app/api/index.js');
const server=createServer(handler);server.listen(0,'127.0.0.1',()=>console.log(JSON.stringify({port:server.address().port})));
