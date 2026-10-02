/** Admin Hermes chat against the local design fixture. The owner grant, the
 * native REST surface and the native gateway WebSocket are intercepted; this
 * proves the UI/transport contract, not live auth or a real Hermes. */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Browser, type Route, type WebSocketRoute } from "playwright";
import { expect } from "@playwright/test";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

const NATIVE = "https://hermes.fixture.test/runtime_web_design/";

test("admin Hermes chat lists archived platform history read-only and runs native turns", { timeout: 180_000 }, async () => {
  const reservation = http.createServer();
  reservation.listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  const port = address.port;
  await new Promise<void>((resolve) => reservation.close(() => resolve()));
  const base = `http://127.0.0.1:${port}`;
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(port), FC_WEB_DESIGN_ADMIN: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", (chunk) => { output += chunk; });
  server.stderr.on("data", (chunk) => { output += chunk; });
  let browser: Browser | undefined;
  try {
    await expect.poll(async () => {
      assert(server.exitCode === null, output);
      try { return (await fetch(`${base}/healthz`)).ok; } catch { return false; }
    }, { timeout: 90_000 }).toBe(true);
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));

    let grants = 0;
    let tickets = 0;
    let persisted = false;
    const listQueries: URLSearchParams[] = [];
    const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
    let socket: WebSocketRoute | undefined;
    let connections = 0;
    let handle = "";

    await page.route("**/api/agents/*/hermes-access", async (route) => {
      assert.equal(route.request().method(), "POST");
      grants++;
      await route.fulfill({ json: { baseUrl: NATIVE, accessToken: "synthetic-browser-test", expiresAt: 100 } });
    });
    const fulfill = (route: Route, json: unknown, status = 200) => route.fulfill({
      status,
      json,
      headers: { "access-control-allow-origin": base, "access-control-allow-headers": "authorization" },
    });
    await page.route(`${NATIVE}**`, async (route) => {
      const request = route.request();
      if (request.method() === "OPTIONS") { await fulfill(route, {}); return; }
      assert.equal(request.headers().authorization, "Bearer synthetic-browser-test");
      assert.equal(request.headers().cookie, undefined);
      const url = new URL(request.url());
      const path = url.pathname.slice(new URL(NATIVE).pathname.length);
      if (path === "api/auth/ws-ticket" && request.method() === "POST") {
        tickets++;
        await fulfill(route, { ticket: `ticket_${"x".repeat(20)}_${tickets}`, ttl_seconds: 30 });
        return;
      }
      if (path === "api/sessions") {
        listQueries.push(url.searchParams);
        await fulfill(route, {
          total: persisted ? 2 : 1,
          sessions: [
            ...(persisted ? [{
              id: "stored-1", source: "desktop", title: "First conversation", preview: "First answer",
              chat_id: null, started_at: 3, last_active: 3, message_count: 2, archived: 0,
            }] : []),
            {
              id: "fc-old", source: "finitechat", title: null, preview: "Old question",
              chat_id: "room_owner", thread_id: "segment_1", started_at: 1, last_active: 2, message_count: 2, archived: 1,
            },
          ],
        });
        return;
      }
      if (path === "api/sessions/fc-old/messages") {
        assert.equal(url.searchParams.get("order"), "latest");
        assert.equal(url.searchParams.get("include_compacted"), "true");
        await fulfill(route, { messages: [
          { id: 1, role: "user", content: "Old question", timestamp: 1 },
          { id: 2, role: "assistant", content: "Old answer", timestamp: 2 },
        ] });
        return;
      }
      if (path === "api/sessions/stored-1/messages") {
        await fulfill(route, { messages: [
          { id: 3, role: "user", content: "First prompt", timestamp: 3 },
          { id: 4, role: "assistant", content: "First answer", timestamp: 3 },
        ] });
        return;
      }
      await fulfill(route, { detail: "unexpected" }, 404);
    });

    const event = (type: string, payload: Record<string, unknown> = {}) => socket?.send(JSON.stringify({
      jsonrpc: "2.0", method: "event", params: { type, session_id: handle, payload },
    }));
    await page.routeWebSocket(/^wss:\/\/hermes\.fixture\.test\/runtime_web_design\/api\/ws$/u, (route) => {
      socket = route;
      connections++;
      handle = `handle-${connections}`;
      route.onMessage((raw) => {
        const request = JSON.parse(String(raw));
        calls.push(request);
        let result: unknown = {};
        if (request.method === "session.create") result = { session_id: handle, stored_session_id: "stored-1" };
        if (request.method === "session.resume") result = { session_id: handle, messages: [] };
        if (request.method === "prompt.submit") {
          assert.equal(request.params.session_id, handle);
          event("message.start");
          event("message.delta", { text: "First " });
          persisted = true;
          event("sessions.changed");
          setTimeout(() => {
            event("message.delta", { text: "answer" });
            event("message.complete", { text: "First answer", status: "complete" });
            route.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { status: "streaming" } }));
          }, 100);
          return;
        }
        route.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
      });
    });

    await page.goto(`${base}/dashboard/machines/runtime_web_design/hermes-chat`);
    await expect(page.getByRole("link", { name: "Hermes chat" })).toHaveAttribute("aria-current", "page");
    await expect.poll(() => connections).toBe(1);
    assert.ok(listQueries.length > 0);
    assert.equal(listQueries[0]!.get("archived"), "include");
    assert.equal(listQueries[0]!.get("order"), "recent");

    // Archived Finite Chat history is listed and readable, but not writable.
    await page.getByText("Finite Chat", { exact: true }).first().waitFor();
    await page.getByRole("button", { name: /Old question/ }).first().click();
    const workspace = page.locator(".finite-chat__workspace");
    await expect(workspace.getByText("Old answer", { exact: true })).toHaveCount(1);
    const composer = page.getByRole("textbox", { name: "Message your agent" });
    await expect(composer).toBeDisabled();
    await expect(composer).toHaveAttribute("placeholder", /read-only here/);
    await expect(page.getByRole("button", { name: "Attach files", exact: true })).toHaveCount(0);

    // A native chat starts from Home, streams, and materializes as a stored session.
    await page.getByRole("button", { name: "New chat", exact: true }).click();
    await expect(composer).toBeEnabled();
    await composer.fill("First prompt");
    await composer.press("Enter");
    await expect(composer).toHaveValue("");
    await expect(workspace.getByText("First answer", { exact: true })).toHaveCount(1);
    await expect(workspace.getByText("First prompt", { exact: true })).toHaveCount(1);
    assert.equal(calls.filter((call) => call.method === "session.create").length, 1);
    await expect(page.getByRole("button", { name: /First conversation/ }).first()).toBeVisible();

    // Each connection gets its own grant and single-use ticket.
    const ticketsBefore = tickets;
    await socket!.close({ code: 1011, reason: "test disconnect" });
    await expect.poll(() => connections).toBe(2);
    assert.equal(tickets, ticketsBefore + 1);
    assert.ok(grants >= tickets);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    if (server.exitCode === null) await once(server, "exit");
  }
});
