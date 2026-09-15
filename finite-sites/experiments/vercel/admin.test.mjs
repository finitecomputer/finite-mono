import test from 'node:test';
import assert from 'node:assert/strict';
import { adminCatalog } from './app/lib/admin.js';

test('admin lists safe links, handles empty state, and denies other routes/methods before reading data', async () => {
  const previous = process.env.POC_SITE_BASE_DOMAIN;
  process.env.POC_SITE_BASE_DOMAIN = 'sites.example.net';
  const response = () => ({headers:{}, setHeader(k,v){this.headers[k]=v;}, end(body){this.body=body;}});
  try {
    const res = response();
    await adminCatalog({method:'GET'},res,new URL('https://admin.sites.example.net/'),async () => [{id:'gamma'},{id:'<script>'},{id:'admin'}]);
    assert.match(res.body,/href="https:\/\/gamma.sites.example.net\/"/);
    assert.match(res.body,/1 active site/);
    assert.ok(!res.body.includes('<script>'));
    assert.match(res.headers['Content-Security-Policy'],/form-action 'none'/);
    const empty = response();
    await adminCatalog({method:'GET'},empty,new URL('https://admin.sites.example.net/'),async () => []);
    assert.match(empty.body,/No active sites yet/);
    const head = response();
    await adminCatalog({method:'HEAD'},head,new URL('https://admin.sites.example.net/'),async () => []);
    assert.equal(head.body,undefined);
    for (const [method,path,status] of [['POST','/',405],['GET','/api/control',404],['POST','/_finite/redeem',404],['GET','/api/v2/sites',404]]) {
      await assert.rejects(adminCatalog({method},response(),new URL(path,'https://admin.sites.example.net'),async () => {throw new Error('unexpected database access');}),{status});
    }
  } finally {
    if (previous === undefined) delete process.env.POC_SITE_BASE_DOMAIN;
    else process.env.POC_SITE_BASE_DOMAIN = previous;
  }
});
