#!/usr/bin/env node
// Receives a short-lived Core grant on stdin, never in argv or a URL.
// Uses a real model turn over the same native gateway protocol as the UI.
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';

let input = '';
for await (const chunk of process.stdin) input += chunk;
const { baseUrl, grantEndpoint, ownerToken, previous, proveInterrupt, proveDesktop, upgradeMarker, proveBrowser, prepareCrash, environmentValue } = JSON.parse(input);
async function accessToken() {
  const response = await fetch(grantEndpoint, {
    method: 'POST', headers: { authorization: `Bearer ${ownerToken}` },
    redirect: 'error', signal: AbortSignal.timeout(15000),
  });
  assert.equal(response.status, 200, 'Fresh Core grant failed');
  const grant = await response.json();
  assert.equal(grant.baseUrl, baseUrl, 'Runtime grant target changed');
  return grant.accessToken;
}
  if (proveBrowser) {
    const browser = await import('../../finitecomputer-v2/apps/dashboard/scripts/substrate-browser-proof.mjs');
    await browser.proveBrowser({ baseUrl, grantEndpoint, ownerToken });
  }
let socket;
let activeSession;
const pending = new Map();
let sequence = 0;
let answer;
let terminalPayload;
let onDelta = () => {};
let onToolStart = () => {};
let onToolComplete = () => {};
let onClarify = () => {};
let onApproval = () => {};
let complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
const deadline = setTimeout(() => { answer.reject(new Error('Model turn timed out')); }, 120000);
// Register rejection handling before starting RPCs.
complete.catch(() => {});
async function connect() {
  const ticketResponse = await fetch(new URL('api/auth/ws-ticket', baseUrl), {
    method: 'POST', headers: { authorization: `Bearer ${await accessToken()}` }, redirect: 'error',
    signal: AbortSignal.timeout(15000),
  });
  assert.equal(ticketResponse.status, 200, 'Native ticket exchange failed');
  const { ticket } = await ticketResponse.json();
  socket = new WebSocket(new URL('api/ws', baseUrl).toString().replace(/^https:/, 'wss:'),
    ['hermes-gateway-v1', `hermes-gateway-ticket.${ticket}`]);
  const connection = socket;
  socket.addEventListener('message', ({ data }) => {
    if (socket !== connection) return;
    const frame = JSON.parse(data);
    const request = pending.get(frame.id);
    if (request) {
      pending.delete(frame.id);
      clearTimeout(request.timer);
      if (frame.error) request.reject(new Error('Native gateway RPC failed'));
      else request.resolve(frame.result);
    }
    if (frame.method === 'event' && frame.params?.type === 'approval.request') onApproval(frame.params.payload);
    if (frame.method === 'event' && frame.params?.type === 'clarify.request') onClarify(frame.params.payload);
    if (frame.method === 'event' && frame.params?.type === 'tool.start') onToolStart(frame.params.payload);
    if (frame.method === 'event' && frame.params?.type === 'tool.complete') onToolComplete(frame.params.payload);
    if (frame.method === 'event' && frame.params?.type === 'message.delta' && frame.params.payload?.text) onDelta();
    if (frame.method === 'event' && frame.params?.type === 'message.complete') {
      terminalPayload = frame.params.payload;
      answer.resolve(frame.params.payload?.text ?? '');
    }
  });
  socket.addEventListener('close', () => {
    if (socket !== connection) return;
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.reject(new Error('Native socket closed'));
    }
    pending.clear();
  });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Native socket open timed out')), 15000);
    connection.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
    connection.addEventListener('error', () => { clearTimeout(timer); reject(new Error('Native socket rejected')); }, { once: true });
  });
  assert.equal(connection.protocol, 'hermes-gateway-v1');
}
function rpc(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 20000);
    pending.set(id, { resolve, reject, timer });
    socket.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }));
  });
}
async function verifyFile(file) {
  const url = new URL('api/files/download', baseUrl);
  url.searchParams.set('path', file.path);
  const response = await fetch(url, { headers: { authorization: `Bearer ${await accessToken()}` }, redirect: 'error', signal: AbortSignal.timeout(15000) });
  assert.equal(response.status, 200, 'Attachment download failed');
  const bytes = Buffer.from(await response.arrayBuffer());
  if (file.content !== undefined) assert.equal(bytes.toString(), file.content, 'Attachment bytes did not survive');
  if (file.sha256) assert.equal(createHash('sha256').update(bytes).digest('hex'), file.sha256, 'Desktop screenshot did not survive restart');
  assert.equal((await fetch(url, { redirect: 'error', signal: AbortSignal.timeout(15000) })).status, 401);
  return bytes;
}
try {
  await connect();
  const marker = `finite-proof-${randomBytes(8).toString('hex')}`;
  const created = previous
    ? await rpc('session.resume', { session_id: previous.storedSessionId })
    : await rpc('session.create', { source: 'desktop', cols: 100 });
  activeSession = created.session_id;
  if (previous) {
    assert.ok(created.messages?.some((message) => message.role === 'assistant' && message.text?.includes(previous.marker)),
      'Assistant reply was not recovered after restart');
  }
  if (previous?.file) await verifyFile(previous.file);
  let desktop = previous?.desktop;
  if (desktop) await verifyFile(desktop);
  let crashProof = previous?.crashProof;
  if (crashProof) {
    assert.equal(Boolean(created.running), false, 'Lost worker left native chat permanently busy');
    assert.equal(created.messages?.filter(message => message.role === 'user' && message.text === crashProof.prompt).length, 1,
      'The accepted pre-crash turn was lost or duplicated');
    await verifyFile(crashProof.file);
  }
  let correctionProof = previous?.correctionProof;
  let ordinaryCorrectionProof = previous?.ordinaryCorrectionProof;
  let queuedProof = previous?.queuedProof;
  let modelStopProof = previous?.modelStopProof;
  const assertCorrectionStored = (messages, correction) => {
    assert.equal(messages?.filter(message => message.role === 'user' && message.text?.includes(correction)).length, 1,
      'Accepted correction is missing or duplicated in stored history');
  };
  if (correctionProof) assertCorrectionStored(created.messages, correctionProof);
  if (ordinaryCorrectionProof) assertCorrectionStored(created.messages, ordinaryCorrectionProof);
  if (queuedProof) assertCorrectionStored(created.messages, queuedProof);
  if (modelStopProof) assertCorrectionStored(created.messages, modelStopProof);
  let image = previous?.image;
  if (image) {
    await verifyFile(image);
    assert.ok(created.messages?.some(message => message.role === 'user' && message.text?.includes(`@image:"${image.path}"`)), 'Image reference did not survive restart');
  }
  let imageVerified = false;
  if (process.env.FC_TEST_SUBSTRATE_IMAGES && proveInterrupt && !previous) {
    // References bind images to this turn; native vision owns inference.
    const dataUrl = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAAbUlEQVR4nO3PwQkAUQhDQftv2q3hH9wQGHhnzczOnHZ7/YfyCwDKyy8AKC+/AKC8/AKA8vILAMrLLwAoL78AoLz8AoDy8gsAyssvACjv/sHxh905DQAAAAAAAAAAAAAAAAAAAAAAAAAAAADguQ/7Be0ehXGAqQAAAABJRU5ErkJggg==';
    const uploadedImage = await rpc('file.attach', { session_id: activeSession, name: 'vision-proof.png', data_url: dataUrl });
    assert.ok(uploadedImage.attached && uploadedImage.path?.startsWith('/'));
    image = { path: uploadedImage.path, sha256: createHash('sha256').update(Buffer.from(dataUrl.split(',')[1], 'base64')).digest('hex') };
    await verifyFile(image);
    let visionToolObserved = false;
    onToolStart = tool => { if (tool.name === 'vision_analyze' && tool.args?.image_url === image.path) visionToolObserved = true; };
    await rpc('prompt.submit', { session_id: activeSession, text: `Name the four quadrant colors in this attached image, clockwise from top left. Answer with just the four color names.\n@image:"${image.path}"` });
    assert.deepEqual((await complete).toLowerCase().match(/red|green|yellow|blue/g), ['red', 'green', 'yellow', 'blue']);
    assert.ok(visionToolObserved, 'Image reference did not invoke native vision routing');
    onToolStart = () => {};
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    imageVerified = true;
  }
  const content = `finite attachment ${marker}`;
  const uploaded = await rpc('file.attach', { session_id: created.session_id, name: `${marker}.txt`, data_url: `data:text/plain;base64,${Buffer.from(content).toString('base64')}` });
  assert.ok(uploaded.attached && uploaded.path?.startsWith('/'));
  const file = { path: uploaded.path, content };
  await verifyFile(file);
  const storedSessionId = previous?.storedSessionId ?? created.stored_session_id;
  assert.ok(created.session_id);
  await rpc('prompt.submit', { session_id: created.session_id, text: `Reply with exactly ${marker}. Do not use any tools.\n@file:${file.path}` });
  assert.ok((await complete).includes(marker), 'Model response did not contain the unique prompt marker');
  if (environmentValue) {
    const environmentPath = `/data/agent/${marker}-environment.txt`;
    const command = `printenv FINITE_SUBSTRATE_ENV_PROOF > ${environmentPath}`;
    let environmentRead = false;
    onToolStart = tool => { if (tool.name === 'terminal' && tool.args?.command?.trim() === command) environmentRead = true; };
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    await rpc('prompt.submit', { session_id: activeSession, text: `Use the terminal tool to run exactly: ${command}. Do not set or override the environment variable. Then say done.` });
    await complete;
    assert.ok(environmentRead, 'Agent must read its actual boot environment');
    onToolStart = () => {};
    await verifyFile({ path: environmentPath, content: `${environmentValue}\n` });
  }
  if (upgradeMarker) {
    let imageReadObserved = false;
    onToolStart = tool => { if (tool.name === 'terminal' && tool.args?.command?.trim() === 'cat /runtime/upgrade-proof') imageReadObserved = true; };
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    await rpc('prompt.submit', { session_id: created.session_id, text: 'Use the terminal tool to run cat /runtime/upgrade-proof. Reply with only the exact file contents. Do not create or edit the file.' });
    assert.ok((await complete).includes(upgradeMarker), 'Agent did not boot the expected image');
    onToolStart = null;
    assert.ok(imageReadObserved, 'Agent did not read the image marker with the terminal tool');
  }
  if (proveInterrupt) {
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    const firstDelta = new Promise(resolve => { onDelta = resolve; });
    const toolStarted = new Promise(resolve => { onToolStart = resolve; });
    const longPrompt = 'First say "Reconnect proof is starting." Then use the terminal tool to run exactly: sleep 20. After it finishes, say done.';
    await rpc('prompt.submit', { session_id: created.session_id, text: longPrompt });
    const [, tool] = await Promise.race([Promise.all([firstDelta, toolStarted]), complete.then(() => { throw new Error('Turn finished before reconnect could be tested'); })]);
    assert.equal(tool.name, 'terminal');
    assert.equal(tool.args?.command?.trim(), 'sleep 20');
    const correction = `After the current tool finishes, reply with exactly corrected-${marker}.`;
    const redirected = await rpc('prompt.submit', { session_id: created.session_id, text: correction });
    assert.equal(redirected.status, 'redirected', 'Default busy submit did not redirect the live turn');
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Native disconnect timed out')), 5000);
      socket.addEventListener('close', () => { clearTimeout(timer); resolve(); }, { once: true });
      socket.close();
    });
    await connect(); // every connection gets a new single-use ticket
    const live = await rpc('session.resume', { session_id: storedSessionId });
    activeSession = live.session_id;
    if (!live.running) console.error(JSON.stringify({ sameHandle: live.session_id === created.session_id, hasInflight: Boolean(live.inflight), status: live.status, messageCount: live.messages?.length }));
    assert.equal(live.running, true, 'Turn did not remain active across disconnect');
    assert.ok(live.inflight?.assistant?.length, 'Resume lost the streamed reply prefix');
    assert.equal(live.inflight?.user, longPrompt);
    assert.deepEqual(live.inflight?.corrections, [correction], 'Reconnect lost or duplicated the accepted correction');
    assert.equal(live.inflight?.correction_offsets?.length, 1);
    assert.ok(Number.isInteger(live.inflight.correction_offsets[0]));
    assert.ok(live.inflight.correction_offsets[0] <= Array.from(live.inflight.assistant).length);
    console.error('native busy-submit correction and reconnect snapshot passed');
    const interrupted = await rpc('session.interrupt', { session_id: live.session_id });
    assert.equal(interrupted.status, 'interrupted');
    await complete;
    assert.equal(terminalPayload.status, 'interrupted', 'Provider turn did not reach interrupted terminal state');
    // A terminal event can precede clearing the native running flag. Read the
    // authoritative session; never infer idleness from the RPC acknowledgement.
    let resumed;
    for (let attempt = 0; attempt < 50; attempt++) {
      resumed = await rpc('session.resume', { session_id: storedSessionId });
      if (!resumed.running) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.equal(resumed.running, false, 'Interrupted session remained busy');
    assertCorrectionStored(resumed.messages, correction);
    correctionProof = correction;
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    await rpc('prompt.submit', { session_id: resumed.session_id, text: `Reply with exactly after-interrupt-${marker}. Do not use any tools.` });
    assert.ok((await complete).includes(`after-interrupt-${marker}`), 'Interrupted session could not accept another turn');
    assert.notEqual(terminalPayload.status, 'error');
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    const steeringTool = new Promise(resolve => { onToolStart = resolve; });
    await rpc('prompt.submit', { session_id: activeSession, text: 'Use the terminal tool to run exactly sleep 5, then reply done.' });
    const steering = await Promise.race([steeringTool, complete.then(() => { throw new Error('Turn finished before steering could be tested'); })]);
    assert.equal(steering.name, 'terminal');
    assert.equal(steering.args?.command?.trim(), 'sleep 5');
    const ordinaryCorrection = `Change of plan: cancel the earlier instruction to reply done. After the current tool finishes, reply with exactly ordinary-correction-${marker}. Do not use another tool.`;
    assert.equal((await rpc('prompt.submit', { session_id: activeSession, text: ordinaryCorrection })).status, 'redirected');
    const ordinaryReply = await complete;
    assert.ok(ordinaryReply.includes(`ordinary-correction-${marker}`), `Model did not apply ordinary steering: ${JSON.stringify({ status: terminalPayload.status, reply: ordinaryReply.slice(0, 240) })}`);
    let settled;
    for (let attempt = 0; attempt < 50; attempt++) {
      settled = await rpc('session.resume', { session_id: storedSessionId });
      if (!settled.running) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.equal(settled.running, false, 'Steered turn remained busy');
    assertCorrectionStored(settled.messages, ordinaryCorrection);
    ordinaryCorrectionProof = ordinaryCorrection;
    console.error('ordinary steering stored history passed');
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    const queueTool = new Promise(resolve => { onToolStart = resolve; });
    await rpc('prompt.submit', { session_id: activeSession, text: 'Use the terminal tool to run exactly sleep 5, then reply done.' });
    const queuedTool = await Promise.race([queueTool, complete.then(() => { throw new Error('Turn finished before queueing could be tested'); })]);
    assert.equal(queuedTool.name, 'terminal');
    assert.equal(queuedTool.args?.command?.trim(), 'sleep 5');
    const queuedText = `Reply with exactly queued-${marker}. Do not use tools.`;
    assert.equal((await rpc('prompt.submit', { session_id: activeSession, text: queuedText, queued: true })).status, 'queued');
    await complete;
    let queuedHistory;
    for (let attempt = 0; attempt < 150; attempt++) {
      queuedHistory = await rpc('session.resume', { session_id: storedSessionId });
      if (!queuedHistory.running && queuedHistory.messages?.some(message => message.role === 'assistant' && message.text?.includes(`queued-${marker}`))) break;
      await new Promise(resolve => setTimeout(resolve, 200));
    }
    assert.equal(queuedHistory.running, false, 'Queued turn did not settle');
    assert.ok(queuedHistory.messages?.some(message => message.role === 'assistant' && message.text?.includes(`queued-${marker}`)), 'Queued prompt did not produce a reply');
    assertCorrectionStored(queuedHistory.messages, queuedText);
    queuedProof = queuedText;
    console.error('queued prompt execution and stored history passed');
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    const modelStreaming = new Promise(resolve => { onDelta = resolve; });
    let modelUsedTool = false;
    onToolStart = () => { modelUsedTool = true; };
    await rpc('prompt.submit', { session_id: activeSession, text: 'Write a long, detailed 3000-word essay on the history of mathematics. Do not use tools.' });
    await Promise.race([modelStreaming, complete.then(() => { throw new Error('Model finished before cancellation could be tested'); })]);
    const modelCorrection = `Reply with exactly model-correction-${marker}.`;
    assert.equal((await rpc('prompt.submit', { session_id: activeSession, text: modelCorrection })).status, 'redirected');
    assert.equal((await rpc('session.interrupt', { session_id: activeSession })).status, 'interrupted');
    await complete;
    assert.equal(terminalPayload.status, 'interrupted');
    assert.equal(modelUsedTool, false, 'Cancellation did not exercise the model-only path');
    await new Promise(resolve => setTimeout(resolve, 1000));
    const stoppedModel = await rpc('session.resume', { session_id: storedSessionId });
    assert.equal(stoppedModel.running, false, 'Stop launched the leftover correction as another turn');
    assertCorrectionStored(stoppedModel.messages, modelCorrection);
    assert.ok(!stoppedModel.messages?.some(message => message.role === 'assistant' && message.text?.includes(`model-correction-${marker}`)), 'Stopped correction executed anyway');
    modelStopProof = modelCorrection;
    console.error('model-time correction cancellation and stored history passed');
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    let clarificationResult;
    onToolComplete = tool => { if (tool.name === 'clarify') clarificationResult = tool.result; };
    const questionReady = new Promise(resolve => { onClarify = resolve; });
    await rpc('prompt.submit', { session_id: activeSession, text: 'Call the clarify tool once with a questions batch: first question "Which formats?", choices ["CSV, with headers", "JSON"], multi_select true; second question "What is the proof answer?", free text. Wait for both answers, then reply with exactly the proof answer. Do not guess or use other tools.' });
    const question = await Promise.race([questionReady, complete.then(() => { throw new Error('Agent completed without requesting clarification'); })]);
    assert.ok(question.request_id);
    assert.equal(question.questions?.length, 2);
    const formats = question.questions.find(item => item.question === 'Which formats?');
    assert.equal(formats?.multi_select, true);
    assert.deepEqual(formats.choices.map(choice => choice.replace(/ \(Recommended\)$/, '')), ['CSV, with headers', 'JSON']);
    const formatAnswer = JSON.stringify(formats.choices);
    const partial = await rpc('clarify.respond', { session_id: activeSession, request_id: question.request_id, question_id: formats.qid, answer: formatAnswer });
    assert.equal(partial.status, 'ok');
    assert.equal(partial.remaining.length, 1);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Clarification disconnect timed out')), 5000);
      socket.addEventListener('close', () => { clearTimeout(timer); resolve(); }, { once: true });
      socket.close();
    });
    await connect();
    const waiting = await rpc('session.resume', { session_id: storedSessionId });
    activeSession = waiting.session_id;
    assert.equal(waiting.pending_clarify?.request_id, question.request_id, 'Reconnect lost the pending clarification');
    assert.equal(waiting.pending_clarify.answers?.[formats.qid], formatAnswer, 'Reconnect lost the accepted multi-select answer');
    const prompt = question.questions.find(item => item.question === 'What is the proof answer?');
    assert.equal(prompt.question, 'What is the proof answer?');
    const clarificationAnswer = `clarified-${randomBytes(12).toString('hex')}`;
    const acknowledged = await rpc('clarify.respond', {
      session_id: activeSession, request_id: question.request_id, answer: clarificationAnswer,
      ...(prompt.qid ? { question_id: prompt.qid } : {}),
    });
    assert.equal(acknowledged.status, 'ok');
    assert.ok((await complete).includes(clarificationAnswer), 'Agent did not receive the clarification answer');
    assert.deepEqual(clarificationResult?.responses?.find(item => item.question === 'Which formats?')?.user_response, ['CSV, with headers', 'JSON']);
    onToolComplete = () => {};
    assert.notEqual(terminalPayload.status, 'error');
    const expired = await rpc('clarify.respond', { session_id: activeSession, request_id: question.request_id, answer: 'late duplicate' });
    assert.equal(expired.status, 'expired', 'Answered clarification accepted a late duplicate');
    console.error('native clarification reconnect, answer, and late-duplicate expiry passed');
    const approvalMode = await rpc('config.get', { key: 'approvals.mode' });
    await rpc('config.set', { key: 'approvals.mode', value: 'manual' });
    const approvalContent = `disposable approval fixture ${marker}`;
    try {
      for (const choice of ['deny', 'once']) {
        // Separate actions: the denied command explicitly must not be retried.
        const approvalFile = await rpc('file.attach', { session_id: activeSession, name: `approval-${choice}-${marker}.txt`, data_url: `data:text/plain;base64,${Buffer.from(approvalContent).toString('base64')}` });
        assert.ok(approvalFile.attached && approvalFile.path?.startsWith('/data/agent/'));
        await verifyFile({ path: approvalFile.path, content: approvalContent });
        complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
        complete.catch(() => {});
        const approvalReady = new Promise(resolve => { onApproval = resolve; });
        const command = `rm -rf -- '${approvalFile.path}'`;
        await rpc('prompt.submit', { session_id: activeSession, text: `This is a disposable approval-test file I just uploaded. Use the terminal tool to run exactly: ${command}. Attempt it only once. If permission is denied, report that and stop; do not bypass approval or use another tool.` });
        const request = await Promise.race([approvalReady, complete.then(() => { throw new Error('Agent finished without command approval'); })]);
        assert.ok(request.request_id);
        assert.ok(request.command.includes(approvalFile.path), 'Approval did not name the disposable fixture');
        const resolved = await rpc('approval.respond', { session_id: activeSession, request_id: request.request_id, choice });
        assert.equal(resolved.resolved, 1);
        await complete;
        const duplicate = await rpc('approval.respond', { session_id: activeSession, request_id: request.request_id, choice });
        assert.equal(duplicate.resolved, 0, 'Answered approval accepted a duplicate');
        if (choice === 'deny') await verifyFile({ path: approvalFile.path, content: approvalContent });
        else {
          const url = new URL('api/files/download', baseUrl);
          url.searchParams.set('path', approvalFile.path);
          assert.equal((await fetch(url, { headers: { authorization: `Bearer ${await accessToken()}` }, redirect: 'error', signal: AbortSignal.timeout(15000) })).status, 404, 'Allowed command did not remove its disposable fixture');
        }
      }
    } finally {
      await rpc('config.set', { key: 'approvals.mode', value: approvalMode.value });
    }
    console.error('native approval deny, allow-once, and duplicate rejection passed');
  }
  if (proveDesktop) {
    const html = `<html><body style="background:#1649a5;color:white;font:40px sans-serif"><h1>Finite desktop proof</h1><p>${marker}</p><div style="width:220px;height:180px;background:#41db78"></div></body></html>`;
    const page = await rpc('file.attach', { session_id: created.session_id, name: `${marker}.html`, data_url: `data:text/html;base64,${Buffer.from(html).toString('base64')}` });
    assert.ok(page.attached && page.path?.startsWith('/'));
    const screenshotPath = `/data/agent/${marker}.png`;
    const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
    const desktopScript = `
const {execFileSync, spawn} = require('node:child_process');
const {setTimeout: delay} = require('node:timers/promises');
(async () => {
  const browserPath = execFileSync('find', ['-L', process.env.PLAYWRIGHT_BROWSERS_PATH, '-type', 'f', '-name', 'chrome', '-perm', '/111'], {encoding:'utf8'}).trim().split('\\n')[0];
  if (!browserPath) throw new Error('Desktop Chromium missing');
  const env = {...process.env, DISPLAY: ':99'};
  const browser = spawn(browserPath, ['--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu', '--no-first-run', '--user-data-dir=/tmp/${marker}', '--window-size=1000,700', ${JSON.stringify(`file://${page.path}`)}], {env, stdio:'ignore', detached:true});
  try {
    await delay(3000);
    if (browser.exitCode !== null) throw new Error('Desktop Chromium exited');
    execFileSync('scrot', [${JSON.stringify(screenshotPath)}], {env, timeout:10000});
    console.log(${JSON.stringify(screenshotPath)});
  } finally {
    process.kill(-browser.pid, 'SIGTERM');
  }
})().catch(error => { console.error(error.message); process.exitCode = 1; });
`;
    const desktopFile = await rpc('file.attach', { session_id: activeSession, name: `${marker}.cjs`, data_url: `data:text/plain;base64,${Buffer.from(desktopScript).toString('base64')}` });
    assert.ok(desktopFile.attached && desktopFile.path?.startsWith('/'));
    await verifyFile({ path: desktopFile.path, content: desktopScript });
    const command = `node ${quote(desktopFile.path)}`;
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    let desktopCommandObserved = false;
    onToolStart = tool => {
      if (tool.name === 'terminal' && tool.args?.command?.trim() === command.trim()) desktopCommandObserved = true;
    };
    await rpc('prompt.submit', { session_id: created.session_id, text: `Use the terminal tool to run this exact desktop screenshot check. Do not install software, change the command, or substitute a headless screenshot. After success report the generated screenshot path.\n\n${command}` });
    await complete;
    assert.notEqual(terminalPayload.status, 'error');
    assert.ok(desktopCommandObserved, 'Agent did not execute the specified headed screenshot command');
    const bytes = await verifyFile({ path: screenshotPath });
    assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', 'Desktop screenshot is not PNG');
    assert.equal(bytes.readUInt32BE(16), 1280);
    assert.equal(bytes.readUInt32BE(20), 800);
    desktop = { path: screenshotPath, sha256: createHash('sha256').update(bytes).digest('hex') };
    if (process.env.FC_TEST_SUBSTRATE_ARTIFACT_DIR) {
      const artifact = join(process.env.FC_TEST_SUBSTRATE_ARTIFACT_DIR, `${marker}.png`);
      await writeFile(artifact, bytes, { mode: 0o600, flag: 'wx' });
      desktop.artifact = artifact;
    }
  }
  const sessions = await rpc('session.list', { limit: 100 });
  assert.ok(sessions.sessions.some((session) => session.id === storedSessionId), 'Accepted turn is missing from durable session inventory');
  // Native history owns archive state, including after reconnect/restart.
  for (const archived of [true, false]) {
    const response = await fetch(new URL(`api/sessions/${encodeURIComponent(storedSessionId)}`, baseUrl), {
      method: 'PATCH', redirect: 'error', signal: AbortSignal.timeout(15000),
      headers: { authorization: `Bearer ${await accessToken()}`, 'content-type': 'application/json' },
      body: JSON.stringify({ archived }),
    });
    assert.equal(response.status, 200, 'Native archive mutation failed');
    const inventory = await fetch(new URL('api/sessions?limit=100&archived=include&order=recent', baseUrl), {
      headers: { authorization: `Bearer ${await accessToken()}` }, redirect: 'error', signal: AbortSignal.timeout(15000),
    });
    assert.equal(inventory.status, 200);
    const row = (await inventory.json()).sessions.find((session) => session.id === storedSessionId);
    assert.equal(row?.archived, archived, 'Native archive state did not persist');
  }
  if (crashProof) await verifyFile(crashProof.file);
  if (prepareCrash) {
    const path = `/data/agent/${marker}-crash-once.txt`;
    const command = `echo ${marker} >> ${path}; sleep 300`;
    const prompt = `Run this exact terminal command in the foreground with timeout 600. Do not background it, retry it, or use another tool. After it returns reply exactly completed-${marker}.\n${command}`;
    let started;
    const toolStarted = new Promise(resolve => { started = resolve; });
    onToolStart = tool => {
      if (tool.name === 'terminal' && tool.args?.command === command && !tool.args?.background) started();
    };
    terminalPayload = undefined;
    complete = new Promise((resolve, reject) => { answer = { resolve, reject }; });
    complete.catch(() => {});
    await rpc('prompt.submit', { session_id: activeSession, text: prompt });
    await Promise.race([toolStarted, complete.then(() => { throw new Error('Crash turn finished before its foreground tool started'); })]);
    const file = { path, content: `${marker}\n` };
    let written = false;
    for (let attempt = 0; attempt < 20; attempt++) {
      try { await verifyFile(file); written = true; break; }
      catch { await new Promise(resolve => setTimeout(resolve, 250)); }
    }
    assert(written, 'Foreground tool never produced its pre-crash side effect');
    assert.equal(terminalPayload, undefined, 'Crash fixture is no longer in flight');
    crashProof = { prompt, file };
    // Closing the socket deliberately leaves this test-owned turn running;
    // the Rust harness next kills its worker and exercises normal recovery.
  }
  console.log(JSON.stringify({ nativeModelTurn: true, imageVerified, browserVerified: Boolean(proveBrowser), interruptionVerified: Boolean(proveInterrupt), clarificationVerified: Boolean(proveInterrupt), approvalVerified: Boolean(proveInterrupt), reconnectVerified: Boolean(proveInterrupt), storedSessionId, marker, file, image, desktop, correctionProof, ordinaryCorrectionProof, queuedProof, modelStopProof, crashProof, recoveredAfterRestart: Boolean(previous) }));
} catch (error) {
  // Closing a native socket leaves model work running. Stop a failed proof's
  // own turn before disconnecting, using the normal native control method.
  if (activeSession && socket?.readyState === WebSocket.OPEN) {
    await rpc('session.interrupt', { session_id: activeSession }).catch(() => {});
  }
  throw error;
} finally {
  clearTimeout(deadline);
  socket?.close();
}
