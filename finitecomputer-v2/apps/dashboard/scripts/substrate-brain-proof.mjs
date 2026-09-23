// Real hosted human signer + Brain service + dashboard approval routes.
// The disposable fixture creates the request; agent tool/card rendering is separate.
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';

export function brainProofClient(brain, user) {
  const deviceHeaders = who => ({ authorization: 'Bearer disposable-control-proof',
    'x-finite-workos-user-id': who, 'x-finite-brain-public-origin': brain,
    'content-type': 'application/json' });
  async function provider(operation, input = {}, who = user) {
    return fetch('http://127.0.0.1:18428/v1/brain/identity-provider', {
      method: 'POST', headers: deviceHeaders(who), signal: AbortSignal.timeout(10000),
      body: JSON.stringify({ version: 'finite-brain-identity-provider-v1', operation, input }),
    });
  }
  async function json(response, label) {
    assert.equal(response.status, 200, `${label}: HTTP ${response.status}`);
    return response.json();
  }
  async function signed(method, path, body) {
    const bodyText = body ? JSON.stringify(body) : '';
    const url = `${brain}${path}`;
    const tags = [['u', url], ['method', method], ['nonce', randomBytes(16).toString('hex')]];
    if (bodyText) tags.push(['payload', createHash('sha256').update(bodyText).digest('hex')]);
    const event = await json(await provider('authorizeHttpRequest', { method, url, bodyText,
      eventTemplate: { kind: 27235, created_at: Math.floor(Date.now() / 1000), tags, content: '' },
    }), 'hosted request signature');
    return json(await fetch(url, { method, headers: {
      authorization: `Nostr ${Buffer.from(JSON.stringify(event)).toString('base64')}`,
      'content-type': 'application/json',
    }, body: bodyText || undefined, signal: AbortSignal.timeout(10000) }), 'Brain request');
  }
  return { provider, json, signed };
}

export async function proveFreshBrainApproval(dashboard, brain, user) {
  const { provider, json, signed } = brainProofClient(brain, user);
  const absent = await provider('identifyMember');
  assert.equal(absent.status, 428, 'fresh native user must not have a preinitialized signer');
  await absent.body?.cancel();
  const list = () => fetch(`${dashboard}/api/brain/approvals`, { signal: AbortSignal.timeout(30000) });
  assert.deepEqual((await json(await list(), 'fresh dashboard approvals')).approvals, []);
  const human = await json(await provider('identifyMember'), 'initialized signer');
  const targetUser = `brain-target-${user}`;
  await json(await fetch('http://127.0.0.1:18428/v1/app/state', {
    headers: { authorization: 'Bearer disposable-control-proof', 'x-finite-workos-user-id': targetUser, 'x-finite-brain-public-origin': brain }, signal: AbortSignal.timeout(10000),
  }), 'disposable recipient');
  const target = await json(await provider('identifyMember', {}, targetUser), 'recipient identity');
  const brainId = 'native-approval-proof';
  await signed('POST', '/v1/brains', { brainId, kind: 'organization', name: 'Disposable approval proof' });
  const pending = await signed('POST', `/v1/brains/${brainId}/approval-requests`, {
    action: 'delegation-grant', targetNpubs: [target.npub],
  });
  const cards = (await json(await list(), 'pending dashboard approval')).approvals;
  assert.equal(cards.length, 1);
  assert.equal(cards[0].id, pending.id);
  const approve = payload => fetch(`${dashboard}/api/brain/approvals/approve`, {
    method: 'POST', headers: { origin: dashboard, 'content-type': 'application/json' },
    body: JSON.stringify({ brainId, requestId: pending.id, payload }), signal: AbortSignal.timeout(30000),
  });
  const rejected = await approve({ ...cards[0].payload, nonce: randomBytes(16).toString('hex') });
  assert.ok(rejected.status >= 400, 'a different nonce must not authorize the pending request');
  await rejected.body?.cancel();
  const applied = await json(await approve(cards[0].payload), 'dashboard approval');
  assert.equal(applied.status, 'applied');
  assert.deepEqual(applied.result.grantedNpubs, [target.npub]);
  assert.deepEqual((await json(await list(), 'resolved dashboard approvals')).approvals, []);
  const metadata = await signed('GET', `/v1/brains/${brainId}/metadata`);
  assert.ok(metadata.admins.includes(human.npub) && metadata.admins.includes(target.npub));
  const requests = await signed('GET', `/v1/brains/${brainId}/approval-requests`);
  const resolved = requests.requests.find(request => request.id === pending.id);
  assert.equal(resolved.status, 'approved');
  assert.equal(resolved.resolvedByNpub, human.npub);
  const replay = await approve(cards[0].payload);
  assert.ok(replay.status >= 400, 'a consumed approval must not apply twice');
  await replay.body?.cancel();
  console.error('fresh native user -> hosted signer -> dashboard -> Brain approval and replay guards passed');
}
