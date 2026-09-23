import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { writeFile } from "node:fs/promises";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

test("chat history stays scoped to each tab across freeze, reconnect, and old-service snapshots", { timeout: 120_000 }, async (t) => {
  const reservation = http.createServer().listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  const base = `http://127.0.0.1:${address.port}`;
  await new Promise<void>(resolve => reservation.close(() => resolve()));
  const reset = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "reset"]);
  await once(reset, "exit");
  assert.equal(reset.exitCode, 0);
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(address.port) }, stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", chunk => { output += chunk; });
  server.stderr.on("data", chunk => { output += chunk; });
  let browser: Browser | undefined;
  try {
    for (let attempt = 0; attempt < 600; attempt++) {
      assert.equal(server.exitCode, null, output);
      if (await fetch(`${base}/healthz`).then(r => r.ok).catch(() => false)) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    let nativeRequests = 0;
    context.on("request", request => {
      if (request.url().includes("/hermes-access")) nativeRequests++;
    });
    const a = await context.newPage();
    const errors: string[] = [];
    context.on("page", page => page.on("pageerror", error => errors.push(error.message)));
    a.on("pageerror", error => errors.push(error.message));
    const api = `${base}/api/chat/machines/runtime_web_design/hosted-device`;
    const pagingActions: string[] = [];
    a.on("request", request => {
      if (request.url().endsWith("/actions") && request.postData()?.includes("LoadOlderMessages")) {
        pagingActions.push(request.postData()!);
      }
    });
    const aStreams: string[] = [];
    a.on("request", request => { if (request.url().includes("/updates")) aStreams.push(request.url()); });
    await a.goto(`${base}/dashboard/machines/runtime_web_design/chat`);
    const history = a.getByText("I kept this conversation after the local dashboard restarted.", { exact: true });
    await history.waitFor();
    const cdp = await context.newCDPSession(a);
    await cdp.send("Page.setWebLifecycleState", { state: "frozen" });
    const b = await context.newPage();
    await b.goto(`${base}/dashboard/machines/runtime_web_design/chat`);
    await b.getByRole("button", { name: "New chat in Home", exact: true }).click();
    await b.locator(".finite-chat__topbar").getByText("New chat", { exact: true }).waitFor();
    await b.locator("textarea").fill("History in the other tab");
    await b.getByRole("button", { name: "Send message", exact: true }).click();
    await b.locator(".finite-chat__messages").getByText("History in the other tab", { exact: true }).waitFor();
    await cdp.send("Page.setWebLifecycleState", { state: "active" });
    await a.bringToFront();
    assert(await history.isVisible(), "resuming A must retain its real message, not just its heading");
    assert.equal(await a.locator(".finite-chat__messages").getByText("History in the other tab", { exact: true }).count(), 0);

    // Close the actual fixture streams. A's reconnect must carry A's scope
    // even though B owns the shared Device cursor at that instant.
    const before = aStreams.length;
    const reconnected = a.waitForResponse(response => response.url().includes("/updates") && response.status() === 200);
    const reconnectRequest = a.waitForRequest(request => request.url().includes("/updates"));
    await writeFile("../../../.local-state/web-design-fixture/scenario", "healthy\n");
    await reconnectRequest;
    assert(aStreams.length > before);
    assert.equal(new URL(aStreams.at(-1)!).searchParams.get("chat_id"), "chat_design");
    await reconnected;
    assert(await history.isVisible());
    const sent = await b.request.post(`${api}/actions`, { data: { SendChatMessage: {
      room_id: "room_design", topic_id: "topic_design", chat_id: "chat_design", text: "New message in A after reconnect",
    } } });
    assert(sent.ok());
    await a.locator(".finite-chat__messages").getByText("New message in A after reconnect", { exact: true }).waitFor();
    assert(await b.locator(".finite-chat__messages").getByText("History in the other tab", { exact: true }).isVisible());
    assert.equal(await b.locator(".finite-chat__messages").getByText("New message in A after reconnect", { exact: true }).count(), 0);

    // A successfully accepted send stays successful if the following view read fails.
    await a.route("**/hosted-device/state?*", route => route.fulfill({ status: 503, json: { error: "View temporarily unavailable" } }), { times: 1 });
    await a.locator("textarea").fill("Accepted despite refresh failure");
    await a.getByRole("button", { name: "Send message", exact: true }).click();
    await a.getByRole("paragraph").filter({ hasText: "Accepted despite refresh failure" }).waitFor();
    await a.waitForFunction(() => document.querySelector("textarea")?.value === "");

    await t.test("send feedback belongs to the action, not a later transcript read", async () => {
      // A different tab can change these Device-wide fields after our text
      // action succeeds but before its scoped transcript refresh completes.
      await a.route("**/hosted-device/state?*", async route => {
        const response = await route.fetch();
        await route.fulfill({ response, json: {
          ...await response.json(),
          status: "attachment unavailable",
          toast: "Attachment failed in another tab",
        } });
      }, { times: 1 });
      await a.locator("textarea").fill("Accepted despite another tab's attachment error");
      await a.getByRole("button", { name: "Send message", exact: true }).click();
      await a.locator(".finite-chat__messages").getByText("Accepted despite another tab's attachment error", { exact: true }).waitFor();
      await a.waitForFunction(() => document.querySelector("textarea")?.value === "", undefined, { timeout: 3_000 });
      assert.equal(await a.getByText("Attachment failed in another tab", { exact: true }).count(), 0);

      // Attachment failures can return HTTP 200 with a refusal in AppState.
      // A later successful view read must not discard the failed draft/files.
      const healthy = await (await a.request.get(`${api}/state?room_id=room_design&topic_id=topic_design&chat_id=chat_design`)).json();
      await a.route("**/hosted-device/attachments", route => route.fulfill({
        status: 200, json: { ...healthy, status: "attachment unavailable", toast: "This attachment was refused" },
      }), { times: 1 });
      await a.locator('input[type="file"]').setInputFiles({
        name: "retry-me.txt", mimeType: "text/plain", buffer: Buffer.from("Keep this file until its own send succeeds."),
      });
      await a.locator("textarea").fill("Keep my failed attachment draft");
      await a.getByRole("button", { name: "Send message", exact: true }).click();
      await a.getByText("This attachment was refused", { exact: true }).waitFor({ timeout: 3_000 });
      assert.equal(await a.locator("textarea").inputValue(), "Keep my failed attachment draft");
      assert(await a.getByRole("button", { name: "Remove retry-me.txt", exact: true }).isVisible());
      assert.equal(await a.getByRole("paragraph").filter({ hasText: "Keep my failed attachment draft" }).count(), 0);
      await a.getByRole("button", { name: "Remove retry-me.txt", exact: true }).click();
      await a.locator("textarea").fill("");
      await a.getByRole("button", { name: "Dismiss", exact: true }).click();
    });

    await t.test("loaded history survives live messages and reconnect without growing another tab's view", async () => {
      for (let index = 0; index < 65; index++) {
        const response = await a.request.post(`${api}/actions`, { data: { SendChatMessage: {
          room_id: "room_design", topic_id: "topic_design", chat_id: "chat_design", text: `Loaded history ${index}`,
        } } });
        assert(response.ok());
      }
      const sameChatTab = await context.newPage();
      await sameChatTab.goto(`${base}/dashboard/machines/runtime_web_design/chat`);
      await sameChatTab.getByRole("button", { name: "Design review", exact: true }).click();
      await sameChatTab.locator(".finite-chat__messages").getByText("Loaded history 64", { exact: true }).waitFor();
      assert.equal(await sameChatTab.locator(".finite-chat__message").count(), 50);
      // Restore B as the shared saved selection while both A tabs keep their views.
      await b.getByRole("button", { name: "New chat", exact: true }).first().click();
      await b.locator(".finite-chat__messages").getByText("History in the other tab", { exact: true }).waitFor();
      await a.getByRole("button", { name: "Load earlier messages", exact: true }).click();
      const oldest = a.locator(".finite-chat__messages").getByText("Loaded history 0", { exact: true });
      await oldest.waitFor();
      assert.deepEqual(pagingActions, [], "paging must use a scoped read, not a shared Device action");
      assert.equal(await sameChatTab.locator(".finite-chat__message").count(), 50,
        "another tab on the same Chat must keep its own window");
      await sameChatTab.close();

      const response = await b.request.post(`${api}/actions`, { data: { SendChatMessage: {
        room_id: "room_design", topic_id: "topic_design", chat_id: "chat_design", text: "New arrival after loading history",
      } } });
      assert(response.ok());
      await a.locator(".finite-chat__messages").getByText("New arrival after loading history", { exact: true }).waitFor();
      assert.equal(await oldest.count(), 1, "a live arrival must retain the oldest Message the user loaded");

      const reconnect = a.waitForResponse(r => r.url().includes("/updates") && r.status() === 200);
      await writeFile("../../../.local-state/web-design-fixture/scenario", "healthy\n");
      await reconnect;
      assert.equal(await oldest.count(), 1, "a reconnect must retain the expanded window");
      assert(await b.locator(".finite-chat__messages").getByText("History in the other tab", { exact: true }).isVisible());
      assert.equal(await b.getByText("Loaded history 0", { exact: true }).count(), 0);

      // Grow beyond another page before paging again. The next page starts
      // from the retained range, not the old numeric request limit.
      for (let index = 0; index < 41; index++) {
        const arrival = await b.request.post(`${api}/actions`, { data: { SendChatMessage: {
          room_id: "room_design", topic_id: "topic_design", chat_id: "chat_design", text: `Later arrival ${index}`,
        } } });
        assert(arrival.ok());
      }
      await a.locator(".finite-chat__messages").getByText("Later arrival 40", { exact: true }).waitFor();
      assert.equal(await oldest.count(), 1);
      await a.getByRole("button", { name: "Load earlier messages", exact: true }).click();
      await history.waitFor();
      assert.equal(await a.getByRole("button", { name: "Load earlier messages", exact: true }).count(), 0);
    });

    // A predecessor service ignores view query parameters. Its wrong-chat
    // baseline must not erase A; explicit navigation remains usable.
    const legacy = await (await b.request.get(`${api}/state`)).json();
    assert.notEqual(legacy.selected_chat_id, "chat_design");
    await a.route("**/hosted-device/state?*", async route => {
      const response = await route.fetch({ url: route.request().url().split("?")[0] });
      await route.fulfill({ response });
    });
    await a.route("**/hosted-device/updates?*", route => route.fulfill({
      contentType: "text/event-stream", body: `event: state\ndata: ${JSON.stringify(legacy)}\n\n`,
    }));
    const legacyRequest = a.waitForRequest(request => request.url().includes("/updates"));
    await writeFile("../../../.local-state/web-design-fixture/scenario", "healthy\n");
    await legacyRequest;
    await a.waitForTimeout(1500);
    assert(await a.locator(".finite-chat__messages").getByText("Later arrival 40", { exact: true }).isVisible(), "a predecessor's unscoped snapshot cannot blank retained history");
    await a.getByRole("button", { name: "New chat", exact: true }).first().click();
    await a.locator(".finite-chat__messages").getByText("History in the other tab", { exact: true }).waitFor();
    await a.getByRole("button", { name: "Design review", exact: true }).click();
    await a.locator(".finite-chat__messages").getByText("Later arrival 40", { exact: true }).waitFor();
    assert.equal(nativeRequests, 0, "Core responses without the native capability must retain Finite Chat");
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
  }
});
