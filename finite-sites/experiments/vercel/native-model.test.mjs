import test from 'node:test';
import assert from 'node:assert/strict';
import {randomBytes} from 'node:crypto';
import {schnorr} from './app/node_modules/@noble/curves/secp256k1.js';
import {verifyNative,viewerRequest} from './app/lib/native.js';
import {hash} from './app/lib/model.js';
const secret=randomBytes(32),pubkey=Buffer.from(schnorr.getPublicKey(secret)).toString('hex'),now=Math.floor(Date.now()/1000),url='https://example.test/api/v2/projects/init';
function header(body,overrides={}){
 const e={pubkey,created_at:now,kind:27235,tags:[['u',url],['method','POST'],['payload',hash(body)]],content:'',...overrides};
 e.id=hash(JSON.stringify([0,e.pubkey,e.created_at,e.kind,e.tags,e.content]));e.sig=Buffer.from(schnorr.sign(Buffer.from(e.id,'hex'),secret)).toString('hex');return 'Nostr '+Buffer.from(JSON.stringify(e)).toString('base64');
}
test('native auth verifies exact bytes and rejects mismatched request/signature/time',()=>{
 for(const body of ['', '{}', ' { "x": "✓" }\n']){
  const auth=header(body);assert.equal(verifyNative(auth,url,'POST',body,now),pubkey);
  assert.throws(()=>verifyNative(auth,url+'x','POST',body,now));assert.throws(()=>verifyNative(auth,url,'GET',body,now));assert.throws(()=>verifyNative(auth,url,'POST',body+' ',now));
 }
 for(const overrides of [{kind:1},{created_at:now-61},{created_at:now+61},{content:'x'},{tags:[['u',url],['u',url],['method','POST'],['payload',hash('{}')]]}])assert.throws(()=>verifyNative(header('{}',overrides),url,'POST','{}',now));
 const decoded=JSON.parse(Buffer.from(header('{}').slice(6),'base64'));decoded.sig='0'.repeat(128);assert.throws(()=>verifyNative('Nostr '+Buffer.from(JSON.stringify(decoded)).toString('base64'),url,'POST','{}',now));
});
test('native return path and nonce are bounded and same-origin',()=>{
 const body={purpose:'finite_site_view_session',return_to:'/report?x=1',client:'finite-dashboard',nonce:'a'.repeat(16)};
 assert.equal(viewerRequest(JSON.stringify(body)).return_to,body.return_to);
 for(const extra of [{return_to:'//evil.test'},{return_to:'/\\evil.test'},{nonce:'short'},{admin:true},{purpose:'publish'}])assert.throws(()=>viewerRequest(JSON.stringify({...body,...extra})));
});
