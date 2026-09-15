import test from 'node:test';
import assert from 'node:assert/strict';
import { manifest, resolveHost, hash, validPath } from './app/lib/model.js';
const env={POC_CONTROL_HOST:'control.example.com',POC_SITE_BASE_DOMAIN:'sites.example.net',POC_SITE_HOSTS:'{"alpha":"alpha-test.vercel.app"}'};
test('wildcard routing only accepts one valid tenant label; aliases are exact',()=>{
  assert.deepEqual(resolveHost('beta.sites.example.net',env),{site:'beta',host:'beta.sites.example.net'});
  assert.equal(resolveHost('ALPHA-TEST.vercel.app',env).site,'alpha');
  assert.equal(resolveHost('control.example.com',env).control,true);
  for (const host of ['alpha.beta.sites.example.net','alpha.sites.example.net.attacker.com','sites.example.net','api.sites.example.net','alpha-test-other.vercel.app','unmapped.vercel.app','beta.sites.example.net:443','-abc.sites.example.net','abc-.sites.example.net']) assert.equal(resolveHost(host,env),null,host);
});
test('manifest identity is stable; changed bytes create another version',()=>{
  const input={commit:'a'.repeat(40),deployPath:'site',files:[{path:'index.html',size:1,sha256:hash('a')},{path:'assets/a.txt',size:1,sha256:hash('b')}]};
  assert.equal(manifest(input).version,manifest({...input,files:[...input.files].reverse()}).version);
  assert.notEqual(manifest(input).version,manifest({...input,commit:'b'.repeat(40)}).version);
  for (const files of [[],[...input.files,input.files[0]],[{...input.files[0],size:1048577}],[{...input.files[0],path:'../index.html'}]]) assert.throws(()=>manifest({...input,files}));
});
test('private paths cannot reach platform routes, hidden secrets, or other sites',()=>{
  for (const path of ['index.html','assets/app.js','.well-known/example']) assert.equal(validPath(path),true,path);
  for (const path of ['../beta/index.html','_finite/redeem','a/../b','a//b','.env','a/.git/config','a%2fb','a\\b']) assert.equal(validPath(path),false,path);
});
