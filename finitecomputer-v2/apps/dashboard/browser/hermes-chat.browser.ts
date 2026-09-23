/** Browser interaction proof against native protocol fixtures. The separate
 * Substrate product proof exercises these APIs against real Hermes actors. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

test("native chat pages older history, archives, reconnects, and restores", { timeout: 120_000 }, async () => {
  const reservation = http.createServer().listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  const base = `http://127.0.0.1:${address.port}`;
  await new Promise<void>(resolve => reservation.close(() => resolve()));
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(address.port), FC_WEB_DESIGN_NATIVE_HERMES: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", chunk => { output += chunk; });
  server.stderr.on("data", chunk => { output += chunk; });
  let browser: Browser | undefined;
  try {
    for (let i = 0; i < 600; i++) {
      assert.equal(server.exitCode, null, output);
      if (await fetch(`${base}/healthz`).then(r => r.ok).catch(() => false)) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    const session = { id: "stored-one", title: "Native history", preview: "Durable reply", started_at: 100, message_count: 2, source: "desktop", archived: false };
    const newerSessions = Array.from({ length: 100 }, (_, index) => ({ ...session, id: `newer-${index}`, title: `Newer conversation ${index}` }));
    const offsets: number[] = [];
    const mutations: boolean[] = [];
    const prompts: string[] = [];
    let requesterAvailable = true;
    let requesterRequests = 0;
    const requesters: unknown[] = [];
    let issuedRequester: Record<string, unknown> | undefined;
    let rejectUpload = true;
    let rejectOlderPage = false;
    let retainedFailure = false;
    let failTurn = true;
    let holdTurn = false;
    let interruptCalls = 0;
    let finishInterrupted: (() => void) | undefined;
    let disconnect: (() => void) | undefined;
    let continueReply: (() => void) | undefined;
    let heldAnswer = "Working 🚀 partial reply";
    let persistHeldUser = false;
    const corrections: string[] = [];
    let connections = 0;
    let legacyCalls = 0;
    type Approval = { request_id: string; command: string; choices: string[] };
    let pendingApproval: Approval | null = null;
    let queuedApproval: Approval | null = null;
    let emitApproval: (() => void) | undefined;
    const approvalResponses: string[] = [];
    let rejectApproval = true;
    let pendingClarify: { request_id: string; questions: { qid: string; question: string; choices?: string[]; multi_select?: boolean }[]; answers: Record<string, string> } | null = null;
    let emitClarify: (() => void) | undefined;
    let expireClarify: ((requestId: string) => void) | undefined;
    let rejectClarify = true;
    const clarificationAnswers: string[] = [];
    let brainApproved = false;
    let liveBrainRequest = false;
    await page.route("**/api/brain/approvals/approve", route => {
      assert.deepEqual(route.request().postDataJSON(), { brainId: "brain-1", requestId: "approval-1", payload: null });
      brainApproved = true;
      return route.fulfill({ json: { artifactId: "fixture-artifact" } });
    });
    await page.route("**/api/brain/approvals", route => route.fulfill({ json: { approvals: brainApproved ? [] : [{
      id: "approval-1", brainId: "brain-1", brainName: "Native Brain",
      action: "invite-commit", expiresAt: Math.floor(Date.now() / 1000) + 900,
      requestedByNpub: "fixture", createdAt: "2026-09-22T00:00:00Z", payload: null,
    }, ...(liveBrainRequest ? [{ id: "approval-live", brainId: "brain-1", brainName: "Native Brain", action: "invite-commit", expiresAt: Math.floor(Date.now() / 1000) + 900, payload: null }] : [])] } }));
    await page.route("**/api/chat/**", route => { legacyCalls++; return route.fulfill({ status: 503, json: {} }); });
    await page.route("**/api/agents/*/hermes-access", route => {
      if (route.request().postDataJSON()?.requester === true) {
        requesterRequests++;
        issuedRequester = requesterAvailable ? {
          userId: "a1".repeat(32), email: "owner@example.com",
          sitesAssertion: requesterRequests.toString(16).padStart(64, "0"),
          expiresAt: Math.floor(Date.now() / 1000) + 600,
        } : undefined;
      }
      return route.fulfill({ json: {
        baseUrl: "https://native.fixture.test/runtimes/agent/", accessToken: "synthetic", expiresAt: 100,
        requester: issuedRequester,
      } });
    });
    await page.route("https://native.fixture.test/**", async route => {
      const request = route.request();
      const url = new URL(request.url());
      const headers = { "access-control-allow-origin": base, "access-control-allow-headers": "authorization,content-type", "access-control-allow-methods": "GET,POST,PATCH" };
      if (request.method() === "OPTIONS") return route.fulfill({ headers });
      assert.equal(request.headers().authorization, "Bearer synthetic");
      assert.equal(request.headers().cookie, undefined);
      if (url.pathname.endsWith("/api/auth/ws-ticket")) return route.fulfill({ headers, json: { ticket: "synthetic-once" } });
      if (request.method() === "PATCH") {
        assert(url.pathname.endsWith("/api/sessions/stored-one"));
        session.archived = request.postDataJSON().archived;
        mutations.push(session.archived);
        return route.fulfill({ headers, json: { ok: true, archived: session.archived } });
      }
      assert.equal(url.searchParams.get("archived"), "include");
      assert.equal(url.searchParams.get("order"), "created");
      const offset = Number(url.searchParams.get("offset"));
      offsets.push(offset);
      if (rejectOlderPage && offset === 100) return route.fulfill({ status: 503, headers, json: {} });
      return route.fulfill({ headers, json: { sessions: [...newerSessions, session].slice(offset, offset + 100), total: 101 } });
    });
    await page.routeWebSocket("wss://native.fixture.test/**", socket => {
      connections++;
      emitClarify = () => socket.send(JSON.stringify({ method: "event", params: {
        session_id: "handle-one", type: "clarify.request", payload: pendingClarify,
      } }));
      expireClarify = requestId => socket.send(JSON.stringify({ method: "event", params: {
        session_id: "handle-one", type: "clarify.expire", payload: { request_id: requestId },
      } }));
      emitApproval = () => socket.send(JSON.stringify({ method: "event", params: {
        session_id: "handle-one", type: "approval.request", payload: pendingApproval,
      } }));
      disconnect = () => socket.close();
      continueReply = () => {
        heldAnswer += " continued";
        socket.send(JSON.stringify({ method: "event", params: { session_id: "handle-one", type: "message.delta", payload: { text: " continued" } } }));
      };
      finishInterrupted = () => socket.send(JSON.stringify({ method: "event", params: { session_id: "handle-one", type: "message.complete", payload: { text: heldAnswer, status: "interrupted" } } }));
      socket.onMessage(raw => {
        const request = JSON.parse(String(raw));
        if (request.method === "clarify.respond") {
          assert(pendingClarify);
          assert.equal(request.params.session_id, "handle-one");
          assert.equal(request.params.request_id, pendingClarify.request_id);
          const qid = request.params.question_id;
          assert(pendingClarify.questions.some(question => question.qid === qid));
          clarificationAnswers.push(qid);
          if (qid === "format") assert.deepEqual(JSON.parse(request.params.answer), ["CSV, with headers", "JSON", "Markdown"]);
          if (rejectClarify) socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, error: { code: -32000, message: "Answer temporarily unavailable" } }));
          else {
            pendingClarify.answers[qid] = request.params.answer;
            const remaining = pendingClarify.questions.map(question => question.qid).filter(id => !(id in pendingClarify!.answers));
            if (!remaining.length) pendingClarify = null;
            socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { status: "ok", remaining } }));
          }
          return;
        }
        if (request.method === "approval.respond") {
          assert.equal(request.params.session_id, "handle-one");
          assert.equal(request.params.request_id, pendingApproval?.request_id);
          assert.equal(request.params.all, undefined);
          approvalResponses.push(request.params.choice);
          if (rejectApproval) socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, error: { code: -32000, message: "Approval temporarily unavailable" } }));
          else {
            pendingApproval = queuedApproval;
            queuedApproval = null;
            socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { resolved: 1 } }));
          }
          return;
        }
        if (request.method === "file.attach") {
          assert.equal(request.params.session_id, "handle-one");
          assert.ok(["notes.txt", "photo.png"].includes(request.params.name));
          assert.equal(Buffer.from(request.params.data_url.split(",")[1], "base64").toString(), "Upload content");
          if (rejectUpload && request.params.name === "photo.png") {
            socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, error: { code: -32000, message: "Upload temporarily unavailable" } }));
          } else {
            socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { attached: true, path: `/data/agent/attachments/${request.params.name}` } }));
          }
          return;
        }
        if (request.method === "session.interrupt") {
          assert.equal(request.params.session_id, "handle-one");
          interruptCalls++;
          socket.send(JSON.stringify(interruptCalls === 1
            ? { jsonrpc: "2.0", id: request.id, error: { code: -32000, message: "Interrupt unavailable" } }
            : { jsonrpc: "2.0", id: request.id, result: { status: "interrupted" } }));
          return;
        }
        if (request.method === "prompt.submit") {
          assert.deepEqual(request.params.finite_requester, issuedRequester);
          requesters.push(request.params.finite_requester);
          assert.equal(requesterRequests, requesters.length, "each prompt must fetch fresh attribution");
          prompts.push(request.params.text);
          if (holdTurn && request.params.text === "Use the revised requirement") {
            corrections.push(request.params.text);
            socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { status: "redirected" } }));
            return;
          }
          retainedFailure = failTurn;
          socket.send(JSON.stringify({ method: "event", params: { session_id: "handle-one", type: "message.start" } }));
          if (request.params.text === "Try another turn") {
            liveBrainRequest = true;
            const emit = (type: string, payload: unknown) => socket.send(JSON.stringify({ method: "event", params: { session_id: "handle-one", type, payload } }));
            emit("message.delta", { text: "Preparing the request" });
            emit("tool.start", { tool_id: "brain-tool", name: "terminal", context: "fbrain" });
            const result = { tool_id: "brain-tool", name: "terminal", result: { output: "finite-brain-approval-filed brain=brain-1 request=approval-live" } };
            emit("tool.complete", result);
            emit("tool.complete", result);
          }
          if (holdTurn) {
            socket.send(JSON.stringify({ method: "event", params: { session_id: "handle-one", type: "message.delta", payload: { text: "Working 🚀 partial reply" } } }));
          } else socket.send(JSON.stringify({ method: "event", params: { session_id: "handle-one", type: "message.complete", payload: failTurn ? { text: "Partial model reply", status: "error", error: "Model service unavailable", partial: true } : { text: "Recovered model reply MEDIA:/data/agent/generated/live.csv" } } }));
        }
        const result = request.method === "projects.tree"
          ? { projects: [], scoped_session_ids: [] }
          : request.method === "session.resume"
            ? { session_id: "handle-one", pending_approval: pendingApproval, pending_clarify: pendingClarify, ...(holdTurn ? { running: true, inflight: { user: "Start a long turn", assistant: heldAnswer, streaming: true, corrections, correction_offsets: corrections.map(() => Array.from(heldAnswer).length) } } : {}), ...(retainedFailure ? { inflight: { status: "error", error: "Model service unavailable", assistant: "Partial model reply", corrections: ["Correction before failure"], correction_offsets: [8] } } : {}), messages: [{ role: "tool", name: "terminal", text: "finite-brain-approval-filed brain=brain-1 request=approval-1" }, { role: "assistant", text: "Durable reply\nMEDIA:/data/agent/generated/report.pdf" }, { role: "user", text: "An uploaded file\n@file:/data/agent/attachments/report.txt" }, ...(persistHeldUser ? [{ role: "user", text: "Start a long turn" }] : [])] }
            : {};
        socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
      });
    });
    const url = `${base}/dashboard/machines/runtime_web_design/chat`;
    await page.goto(url);
    await page.getByRole("button", { name: "Native history", exact: true }).waitFor();
    assert.equal(await page.getByRole("button", { name: "New chat", exact: true }).isEnabled(), true,
      "native Home must be a valid target for the shared New chat control");
    await page.getByRole("button", { name: "Native history", exact: true }).click();
    await page.locator(".finite-chat__messages").getByText("Durable reply", { exact: true }).waitFor();
    await page.getByRole("region", { name: "Brain actions" }).getByRole("button", { name: "Approve", exact: true }).waitFor();
    await page.getByRole("button", { name: "Native history", exact: true }).hover();
    await page.getByRole("button", { name: "Archive Native history", exact: true }).click();
    await page.getByRole("button", { name: "Archive", exact: true }).waitFor();
    assert.deepEqual(mutations, [true]);
    assert(offsets.includes(100), "older conversations must be loaded beyond Hermes's first page");
    await page.reload();
    const archive = page.getByRole("button", { name: "Archive", exact: true });
    await archive.waitFor();
    if (await archive.getAttribute("aria-expanded") !== "true") await archive.click();
    await page.getByRole("button", { name: "Native history", exact: true }).hover();
    await page.getByRole("button", { name: "Restore Native history", exact: true }).click();
    await page.getByRole("button", { name: "Archive Native history", exact: true }).waitFor();
    assert.deepEqual(mutations, [true, false]);
    await page.getByRole("button", { name: "Native history", exact: true }).click();
    await page.locator(".finite-chat__messages").getByText("Durable reply", { exact: true }).waitFor();
    await page.getByRole("region", { name: "Brain actions" }).getByRole("button", { name: "Approve", exact: true }).waitFor();
    if (process.env.FC_BROWSER_ARTIFACT_DIR) await page.screenshot({ path: `${process.env.FC_BROWSER_ARTIFACT_DIR}/native-chat-restored.png`, fullPage: true });
    const download = page.getByRole("link", { name: "report.txt", exact: true });
    await download.waitFor();
    assert.equal(await download.getAttribute("href"), "/api/agents/runtime_web_design/hermes-file?path=%2Fdata%2Fagent%2Fattachments%2Freport.txt");
    assert.equal(await page.locator(".finite-chat__messages").getByText("/data/agent/attachments/report.txt", { exact: true }).count(), 0);
    const generated = page.getByRole("link", { name: "report.pdf", exact: true });
    await generated.waitFor();
    assert.equal(await generated.getAttribute("href"), "/api/agents/runtime_web_design/hermes-file?path=%2Fdata%2Fagent%2Fgenerated%2Freport.pdf");
    const composer = page.locator(".finite-chat__composer textarea");
    await composer.fill("Read these notes");
    await page.locator('input[type="file"]').setInputFiles([{ name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("Upload content") }, { name: "photo.png", mimeType: "image/png", buffer: Buffer.from("Upload content") }]);
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    await page.getByText("Upload temporarily unavailable (-32000)", { exact: true }).waitFor();
    assert.equal(await composer.inputValue(), "Read these notes", "failed upload retains the draft");
    assert.deepEqual(prompts, [], "failed upload must not submit a partial message");
    rejectUpload = false;
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    await page.getByRole("link", { name: "notes.txt", exact: true }).waitFor();
    await page.waitForFunction(() => document.querySelector<HTMLTextAreaElement>(".finite-chat__composer textarea")?.value === "");
    assert.deepEqual(prompts, ["Read these notes\n@file:`/data/agent/attachments/notes.txt`\n@image:`/data/agent/attachments/photo.png`"]);
    assert(connections >= 2);
    assert.equal(legacyCalls, 0, "native chat must not read or write Finite Chat history");
    await page.getByText("Agent turn failed: Model service unavailable", { exact: true }).first().waitFor();
    await page.locator(".finite-chat__messages").getByText("Partial model reply", { exact: true }).waitFor();
    await page.reload();
    await page.getByRole("button", { name: "Native history", exact: true }).click();
    await page.getByText("Agent turn failed: Model service unavailable", { exact: true }).first().waitFor();
    assert.equal(await page.locator(".finite-chat__messages").getByText("model reply", { exact: true }).count(), 1,
      "retained turn failure must survive reconnect without duplicate reply rows");
    assert.equal(await page.locator(".finite-chat__messages").getByText("Correction before failure", { exact: true }).count(), 1,
      "a failed turn must retain its accepted correction");
    failTurn = false;
    await composer.fill("Try another turn");
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    await page.locator(".finite-chat__messages").getByText("Recovered model reply", { exact: true }).waitFor();
    await page.getByRole("region", { name: "Brain actions" }).nth(1).getByRole("button", { name: "Approve", exact: true }).waitFor();
    assert.equal(await page.getByRole("region", { name: "Brain actions" }).count(), 2);
    assert.equal(await page.getByText("Agent turn failed: Model service unavailable", { exact: true }).count(), 0);
    holdTurn = true;
    await composer.fill("Start a long turn");
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    const stop = page.getByRole("button", { name: "Stop response", exact: true });
    await stop.waitFor();
    await composer.fill("Unsent followup");
    let submitted = prompts.length;
    const previousConnections = connections;
    assert(disconnect);
    disconnect();
    await page.getByText("Agent chat is unavailable. Reconnecting…", { exact: true }).first().waitFor();
    await stop.waitFor();
    assert(connections > previousConnections);
    assert.equal(prompts.length, submitted, "reconnect must not resubmit the accepted prompt");
    assert.equal(await page.locator(".finite-chat__messages").getByText("Start a long turn", { exact: true }).count(), 1,
      "the accepted user turn must survive reconnect");
    assert.equal(await composer.inputValue(), "Unsent followup");
    assert.equal(await page.locator(".finite-chat__messages").getByText(heldAnswer, { exact: true }).count(), 1);
    persistHeldUser = true;
    await composer.fill("Use the revised requirement");
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    await page.waitForFunction(() => document.querySelector<HTMLTextAreaElement>(".finite-chat__composer textarea")?.value === "");
    submitted++;
    assert.equal(prompts.length, submitted);
    assert.deepEqual(corrections, ["Use the revised requirement"]);
    await composer.fill("Unsent followup");
    disconnect();
    await page.getByText("Agent chat is unavailable. Reconnecting…", { exact: true }).first().waitFor();
    await stop.waitFor();
    assert.equal(prompts.length, submitted);
    assert.equal(await page.locator(".finite-chat__messages").getByText("Start a long turn", { exact: true }).count(), 1,
      "a persisted open user row must not be duplicated by the in-flight snapshot");
    await page.locator(".finite-chat__messages").getByText("Use the revised requirement", { exact: true }).waitFor({ timeout: 5000 });
    assert.equal(await page.locator(".finite-chat__messages").getByText("Use the revised requirement", { exact: true }).count(), 1,
      "accepted mid-turn corrections must survive reconnect exactly once");
    const transcript = await page.locator(".finite-chat__messages").innerText();
    assert(transcript.indexOf(heldAnswer) < transcript.indexOf("Use the revised requirement"),
      "the correction follows the output already seen when it was submitted");
    assert(continueReply);
    continueReply();
    await page.locator(".finite-chat__messages").getByText("continued", { exact: true }).waitFor();
    if (process.env.FC_BROWSER_ARTIFACT_DIR) await page.screenshot({ path: `${process.env.FC_BROWSER_ARTIFACT_DIR}/native-stop-response.png`, fullPage: true });
    await stop.click();
    await page.getByText("Interrupt unavailable (-32000)", { exact: true }).waitFor();
    assert.equal(interruptCalls, 1, "failed interruption must not retry automatically");
    assert.equal(await composer.inputValue(), "Unsent followup");
    await stop.click();
    await page.waitForFunction(() => !document.querySelector('button[aria-label="Stop response"]')?.hasAttribute("disabled"));
    assert.equal(interruptCalls, 2);
    assert(await stop.isVisible(), "interrupt acknowledgement must not invent terminal completion");
    assert(finishInterrupted);
    finishInterrupted();
    await stop.waitFor({ state: "hidden" });
    assert.equal(await page.locator(".finite-chat__messages").getByText("continued", { exact: true }).count(), 1);
    assert.equal(await composer.inputValue(), "Unsent followup");
    pendingApproval = { request_id: "approval-one", command: "fixture-command --dry-run", choices: ["once", "deny"] };
    assert(emitApproval);
    emitApproval();
    const approvalCard = page.getByRole("region", { name: "Command approval" });
    await approvalCard.getByText("fixture-command --dry-run", { exact: true }).waitFor();
    await approvalCard.getByRole("button", { name: "Allow once", exact: true }).click();
    await approvalCard.getByRole("alert").getByText("Approval temporarily unavailable (-32000)", { exact: true }).waitFor();
    assert.deepEqual(approvalResponses, ["once"], "failed approvals must not retry automatically");
    disconnect();
    await page.getByText("Agent chat is unavailable. Reconnecting…", { exact: true }).first().waitFor();
    await approvalCard.getByRole("button", { name: "Deny", exact: true }).waitFor();
    rejectApproval = false;
    await approvalCard.getByRole("button", { name: "Deny", exact: true }).click();
    await approvalCard.waitFor({ state: "hidden" });
    assert.deepEqual(approvalResponses, ["once", "deny"]);
    pendingApproval = { request_id: "approval-two", command: "second-fixture-command", choices: ["once", "deny"] };
    emitApproval();
    await approvalCard.getByRole("button", { name: "Allow once", exact: true }).click();
    await approvalCard.waitFor({ state: "hidden" });
    assert.deepEqual(approvalResponses, ["once", "deny", "once"]);
    assert.equal(await composer.inputValue(), "Unsent followup");
    pendingApproval = { request_id: "older-approval", command: "older-fixture-command", choices: ["once", "deny"] };
    emitApproval();
    await approvalCard.getByText("older-fixture-command", { exact: true }).waitFor();
    queuedApproval = pendingApproval;
    pendingApproval = { request_id: "newer-approval", command: "newer-fixture-command", choices: ["once", "deny"] };
    emitApproval();
    await approvalCard.getByText("newer-fixture-command", { exact: true }).waitFor();
    await approvalCard.getByRole("button", { name: "Deny", exact: true }).click();
    await approvalCard.getByText("older-fixture-command", { exact: true }).waitFor();
    await approvalCard.getByRole("button", { name: "Deny", exact: true }).click();
    await approvalCard.waitFor({ state: "hidden" });
    assert.deepEqual(approvalResponses, ["once", "deny", "once", "deny", "deny"]);
    pendingClarify = { request_id: "clarify-one", questions: [
      { qid: "destination", question: "Where should the report go?" },
      { qid: "format", question: "Which format?", choices: ["CSV, with headers", "JSON"], multi_select: true },
      { qid: "finish", question: "Ready?", choices: ["Yes", "No"] },
    ], answers: {} };
    assert(emitClarify);
    emitClarify();
    const questionCard = page.getByRole("region", { name: "Agent question" });
    await questionCard.getByRole("textbox", { name: "Where should the report go?", exact: true }).fill("Project folder");
    await questionCard.getByRole("button", { name: "Send answer", exact: true }).first().click();
    await questionCard.getByRole("alert").getByText("Answer temporarily unavailable (-32000)", { exact: true }).waitFor();
    assert.deepEqual(clarificationAnswers, ["destination"]);
    rejectClarify = false;
    await questionCard.getByRole("button", { name: "Send answer", exact: true }).first().click();
    await questionCard.getByRole("button", { name: "Update answer", exact: true }).waitFor();
    disconnect();
    await page.getByText("Agent chat is unavailable. Reconnecting…", { exact: true }).first().waitFor();
    await questionCard.getByRole("button", { name: "Update answer", exact: true }).waitFor();
    assert.equal(await questionCard.getByRole("textbox", { name: "Where should the report go?", exact: true }).inputValue(), "Project folder");
    await questionCard.getByRole("checkbox", { name: "CSV, with headers", exact: true }).check();
    await questionCard.getByRole("checkbox", { name: "JSON", exact: true }).check();
    await questionCard.getByRole("textbox", { name: "Which format?", exact: true }).fill("Markdown");
    await questionCard.locator("form").filter({ has: page.getByRole("textbox", { name: "Which format?", exact: true }) }).getByRole("button", { name: "Send answer", exact: true }).click();
    await questionCard.getByRole("button", { name: "Update answer", exact: true }).nth(1).waitFor();
    await page.reload();
    await page.getByRole("button", { name: "Native history", exact: true }).click();
    await questionCard.getByRole("button", { name: "Update answer", exact: true }).nth(1).waitFor();
    assert.equal(await questionCard.getByRole("checkbox", { name: "CSV, with headers", exact: true }).isChecked(), true);
    assert.equal(await questionCard.getByRole("checkbox", { name: "JSON", exact: true }).isChecked(), true);
    assert.equal(await questionCard.getByRole("textbox", { name: "Which format?", exact: true }).inputValue(), "Markdown");
    await questionCard.getByRole("radio", { name: "Yes", exact: true }).check();
    await questionCard.getByRole("button", { name: "Send answer", exact: true }).click();
    await questionCard.waitFor({ state: "hidden" });
    assert.deepEqual(clarificationAnswers, ["destination", "destination", "format", "finish"]);
    // Expiry is scoped to the request, not merely to its session.
    assert(expireClarify);
    pendingClarify = { request_id: "clarify-new", questions: [{ qid: "next", question: "New question?" }], answers: {} };
    emitClarify();
    await questionCard.getByRole("textbox", { name: "New question?", exact: true }).waitFor();
    expireClarify("clarify-one");
    // Wait for a later event on the same socket to render before checking that
    // the earlier stale expiry did not remove the question.
    pendingApproval = { request_id: "expiry-barrier", command: "echo expiry barrier", choices: ["once", "deny"] };
    emitApproval();
    await approvalCard.getByText("echo expiry barrier", { exact: true }).waitFor();
    assert.equal(await questionCard.isVisible(), true, "stale expiry must preserve the newer request");
    pendingClarify = null;
    expireClarify("clarify-new");
    await questionCard.waitFor({ state: "hidden" });
    await approvalCard.getByRole("button", { name: "Deny", exact: true }).click();
    await approvalCard.waitFor({ state: "hidden" });
    rejectOlderPage = true;
    await page.reload();
    await page.getByText("Could not load gateway conversations. Retry load to reconnect.", { exact: true }).first().waitFor();
    assert.equal(await page.getByRole("button", { name: "Newer conversation 0", exact: true }).count(), 0,
      "a failed later page must not publish a partial inventory");
    rejectOlderPage = false;
    await page.reload();
    await page.getByRole("button", { name: "Native history", exact: true }).waitFor();
    holdTurn = false;
    await page.getByRole("button", { name: "Native history", exact: true }).click();
    requesterAvailable = false; // Sites outage must not prevent the next native turn.
    const brainCard = page.getByRole("region", { name: "Brain actions" });
    await brainCard.getByRole("button", { name: "Approve", exact: true }).click();
    await page.locator(".finite-chat__messages").getByText("Approved: invitation approval for Native Brain", { exact: true }).waitFor();
    assert(brainApproved);
    assert.equal(requesters.at(-1), undefined);
    assert.ok(requesters[0]);
    assert.equal(prompts.at(-1), "Approved: invitation approval for Native Brain");
    await page.reload();
    await page.getByRole("button", { name: "Native history", exact: true }).click();
    await brainCard.waitFor();
    assert.equal(await brainCard.getByRole("button", { name: "Approve", exact: true }).count(), 0,
      "a resolved server request cannot reopen through native history projection");
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
  }
});
