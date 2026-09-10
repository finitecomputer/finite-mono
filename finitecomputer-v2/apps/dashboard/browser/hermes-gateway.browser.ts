import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import http from "node:http";
import { test } from "node:test";
import { chromium, type WebSocketRoute } from "playwright";
import { expect } from "@playwright/test";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

// Exercise the real shell, provider, sidebar and composer, with only the
// gateway wire replaced. Event ordering deliberately differs from RPC ordering.
test("gateway first prompt, materialization, refusal, reopen and reconnect", { timeout: 120_000 }, async () => {
  let base = process.env.GATEWAY_BROWSER_TEST_BASE_URL;
  let fixture: ReturnType<typeof spawn> | undefined;
  let output = "";
  if (!base) {
    const listener = http.createServer();
    listener.listen(0, "127.0.0.1");
    await once(listener, "listening");
    const port = (listener.address() as { port: number }).port;
    await new Promise<void>((resolve) => listener.close(() => resolve()));
    base = `http://127.0.0.1:${port}`;
    fixture = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
      env: {
        ...process.env,
        FC_WEB_DESIGN_PORT: String(port),
        NEXT_PUBLIC_HERMES_GATEWAY_WS_URL: "ws://127.0.0.1:19120/api/ws",
        NEXT_PUBLIC_HERMES_GATEWAY_TOKEN: "browser-test-token",
        NEXT_PUBLIC_HERMES_GATEWAY_USERNAME: "",
        NEXT_PUBLIC_HERMES_GATEWAY_PASSWORD: "",
        HERMES_GATEWAY_PROXY_TARGET: "",
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    fixture.stdout?.on("data", (chunk) => { output += chunk; });
    fixture.stderr?.on("data", (chunk) => { output += chunk; });
  }
  let browser: Awaited<ReturnType<typeof chromium.launch>> | undefined;
  try {
    await expect.poll(async () => {
      if (fixture && fixture.exitCode !== null) throw new Error(output);
      try { return (await fetch(`${base}/dashboard/machines/runtime_web_design/gateway-chat`)).status; } catch { return 0; }
    }, { timeout: 60_000 }).toBe(200);
    browser = await chromium.launch(chromiumLaunchOptions());
    const page = await browser.newPage();
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const calls: Array<{ method: string; params: Record<string, unknown> }> = [];
    let socket: WebSocketRoute;
    let connections = 0;
    let persisted = false;
    let rejectPrompt = false;
    let handle = "handle-1";
    const history = [{ role: "user", text: "First prompt" }, { role: "assistant", text: "First answer" }];
    const event = (type: string, payload: Record<string, unknown> = {}) => socket.send(JSON.stringify({
      jsonrpc: "2.0", method: "event", params: { type, session_id: handle, payload },
    }));
    await page.routeWebSocket("ws://127.0.0.1:19120/api/ws*", (route) => {
      socket = route;
      connections++;
      handle = `handle-${connections}`;
      route.onMessage((raw) => {
        const request = JSON.parse(String(raw));
        calls.push(request);
        let result: unknown = {};
        if (request.method === "projects.tree") result = { projects: [], scoped_session_ids: [] };
        if (request.method === "session.list") result = { sessions: persisted ? [{
          id: "stored-1", title: "First conversation", preview: "First answer", started_at: 1, message_count: 2, source: "desktop",
        }] : [] };
        if (request.method === "session.create") result = { session_id: handle, stored_session_id: "stored-1" };
        if (request.method === "session.resume") result = { session_id: handle, messages: history };
        if (request.method === "prompt.submit") {
          if (rejectPrompt) {
            route.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, error: { code: 4001, message: "Model unavailable" } }));
            return;
          }
          assert.equal(request.params.session_id, handle);
          event("message.start");
          event("message.delta", { text: "First " });
          persisted = true;
          event("sessions.changed");
          // Deliver remaining events after list reconciliation, but before the RPC ack.
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
    await page.goto(`${base}/dashboard/machines/runtime_web_design/gateway-chat?prompt=First%20prompt`);
    const composer = page.getByRole("textbox", { name: "Message your agent" });
    await expect(composer).toBeEnabled();
    await expect(page.getByRole("button", { name: "Attach files", exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Start audio recording", exact: true })).toHaveCount(0);
    await expect(composer).toHaveValue("First prompt");
    await composer.press("Enter");
    await expect(composer).toHaveValue("");
    await expect(page.locator(".finite-chat__workspace").getByText("First answer", { exact: true })).toHaveCount(1);
    await expect(page.locator(".finite-chat__workspace").getByText("First prompt", { exact: true })).toHaveCount(1);
    assert.equal(calls.filter((call) => call.method === "session.create").length, 1);
    assert.equal(calls.find((call) => call.method === "session.create")?.params.cwd, undefined);

    rejectPrompt = true;
    await composer.fill("Keep this failed prompt");
    await composer.press("Enter");
    await expect(page.getByText(/Model unavailable/)).toBeVisible();
    await expect(composer).toHaveValue("Keep this failed prompt");
    await expect(page.locator(".finite-chat__messages").getByText("Keep this failed prompt", { exact: true })).toHaveCount(0);

    // A new WebSocket must rebind the selected stored session and fetch history.
    const beforeReconnect = connections;
    await socket!.close({ code: 1011, reason: "test disconnect" });
    await expect.poll(() => connections).toBe(beforeReconnect + 1);
    await expect.poll(() => calls.filter((call) => call.method === "session.resume").length).toBeGreaterThan(0);
    await expect(page.locator(".finite-chat__workspace").getByText("First answer", { exact: true })).toHaveCount(1);
    const resumes = calls.filter((call) => call.method === "session.resume");
    assert.equal(resumes.at(-1)?.params.session_id, "stored-1");
    assert.equal(calls.filter((call) => call.method === "session.create").length, 1);
    await expect(page.getByRole("button", { name: "Archive First conversation", exact: true })).toHaveCount(0);
    assert.deepEqual(errors, []);
  } finally {
    await browser?.close();
    fixture?.kill("SIGTERM");
    if (fixture && fixture.exitCode === null) await once(fixture, "exit");
  }
});
