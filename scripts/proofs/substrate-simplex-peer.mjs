#!/usr/bin/env node
// Talks only to a disposable human-side SimpleX daemon, never the agent's WS.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
let input = '';
for await (const chunk of process.stdin) input += chunk;
const { port, address, contactId, mode } = JSON.parse(input);
let socket;
for (let attempt = 0; attempt < 40; attempt++) {
  try {
    socket = new WebSocket(`ws://127.0.0.1:${port}`);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', reject, { once: true });
    });
    break;
  } catch { socket?.close(); await delay(250); }
}
assert.equal(socket?.readyState, WebSocket.OPEN, 'Disposable SimpleX client unavailable');
let sequence = 0;
const pending = new Map();
const events = [];
let receivedReplies = 0;
let markerSplitByWhitespace = false;
socket.addEventListener('message', ({ data }) => {
  const event = JSON.parse(data);
  const request = pending.get(event.corrId);
  if (request) { pending.delete(event.corrId); clearTimeout(request.timer); request.resolve(event.resp); }
  else { events.push(event.resp); if (events.length > 256) events.shift(); }
});
async function command(cmd) {
  const corrId = String(++sequence);
  const response = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(corrId); reject(new Error('SimpleX command timed out')); }, 30000);
    pending.set(corrId, { resolve, timer });
    socket.send(JSON.stringify({ corrId, cmd }));
  });
  assert.ok(!['chatCmdError', 'chatError'].includes(response.type), 'SimpleX command rejected');
  return response;
}
async function until(predicate, timeout = 90000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    while (events.length) {
      const event = events.shift();
      if (predicate(event)) return event;
    }
    await delay(100);
  }
  return null;
}
function receivedMarker(event, marker) {
  if (event?.type !== 'newChatItems') return false;
  let matched = false;
  for (const { chatItem } of event.chatItems) {
    if (chatItem?.chatDir?.type !== 'directRcv') continue;
    const text = chatItem.content?.msgContent?.text;
    if (typeof text !== 'string') continue;
    receivedReplies++;
    matched ||= text.includes(marker);
    markerSplitByWhitespace ||= !text.includes(marker) && text.replace(/\s/g, '').includes(marker);
  }
  return matched;
}
try {
  let contact = contactId;
  if (mode === 'pair') {
    await command(`/connect ${address}`);
    const connected = await until(event => event.type === 'contactConnected');
    assert.ok(connected, 'SimpleX contact connection timed out');
    contact = connected.contact.contactId;
  }
  assert.ok(Number.isSafeInteger(contact) && contact > 0);
  const marker = `simplex-proof-${randomBytes(12).toString('hex')}`;
  await command(`/_send @${contact} json ${JSON.stringify([{ msgContent: { type: 'text', text: `Reply with exactly ${marker}. Do not use tools.` } }])}`);
  const reply = await until(event => receivedMarker(event, marker), mode === 'pair' ? 5000 : 90000);
  if (mode === 'pair') assert.equal(reply, null, 'Unapproved contact received the requested agent response');
  else assert.ok(reply, `Approved SimpleX exact reply timed out (receivedReplies=${receivedReplies}, markerSplitByWhitespace=${markerSplitByWhitespace})`);
  console.log(JSON.stringify({ contactId: contact, replyVerified: mode !== 'pair' }));
} finally {
  for (const request of pending.values()) clearTimeout(request.timer);
  socket.close();
}
