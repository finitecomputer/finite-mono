import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Browser } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

test("sidebar drag, topic moves, saved order, failures, and keyboard/touch controls", { timeout: 150_000 }, async () => {
  const reservation = http.createServer().listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const address = reservation.address();
  assert(address && typeof address !== "string");
  const base = `http://127.0.0.1:${address.port}`;
  await new Promise<void>((resolve) => reservation.close(() => resolve()));
  const reset = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "reset"]);
  await once(reset, "exit");
  assert.equal(reset.exitCode, 0);
  const server = spawn(process.execPath, ["--import", "tsx", "scripts/web-design-fixture.ts", "serve"], {
    env: { ...process.env, FC_WEB_DESIGN_PORT: String(address.port) }, stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  server.stdout.on("data", (chunk) => { output += chunk; });
  server.stderr.on("data", (chunk) => { output += chunk; });
  let browser: Browser | undefined;
  try {
    for (let attempt = 0; attempt < 600; attempt++) {
      assert.equal(server.exitCode, null, output);
      if (await fetch(`${base}/healthz`).then((r) => r.ok).catch(() => false)) break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    browser = await chromium.launch({ headless: true, ...chromiumLaunchOptions() });
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const api = `${base}/api/chat/machines/runtime_web_design/hosted-device`;
    const url = `${base}/dashboard/machines/runtime_web_design/chat`;
    await page.goto(url);
    const history = page.getByText("I kept this conversation after the local dashboard restarted.", { exact: true });
    await history.waitFor();
    assert.equal(await page.locator("[data-nextjs-dialog]").count(), 0);
    const home = page.locator('[data-topic-id="home"]');
    const general = page.locator('[data-topic-id="topic_design"]');
    const design = page.locator('[data-chat-id="chat_design"]');
    const ids = (topic = home) => topic.locator(".finite-chat__thread-row").evaluateAll((rows) => rows.map((row) => row.getAttribute("data-chat-id")));
    for (const intent_key of ["first", "second"]) {
      const response = await page.request.post(`${api}/actions`, { data: { StartTopicChatIntent: { room_id: "room_design", topic_id: "home", intent_key, reason: null } } });
      assert(response.ok());
    }
    await home.locator(".finite-chat__thread-row").nth(1).waitFor();
    await design.locator(".finite-chat__thread-open").click();
    await history.waitFor();
    const initial = await ids();
    const first = home.locator(`[data-chat-id="${initial[0]}"]`);
    const second = home.locator(`[data-chat-id="${initial[1]}"]`);
    await second.getByRole("button", { name: "Move New chat", exact: true }).dragTo(first);
    await page.waitForFunction((expected) => Array.from(document.querySelectorAll('[data-topic-id="home"] .finite-chat__thread-row')).map((row) => row.getAttribute("data-chat-id")).join() === expected, [...initial].reverse().join());
    assert(await history.isVisible(), "background reordering must retain the active transcript");

    // Cross-topic drag into collapsed Home expands it; reload retains the selected chat.
    await page.getByRole("button", { name: "Collapse Home", exact: true }).click();
    await design.getByRole("button", { name: "Move Design review", exact: true }).dragTo(home.locator(".finite-chat__folder-header"));
    await home.locator('[data-chat-id="chat_design"]').waitFor();
    assert.equal(await design.locator('[aria-current="page"]').count(), 1);
    assert(await history.isVisible());
    await page.reload();
    await history.waitFor();
    assert.deepEqual(await ids(), [...initial].reverse().concat("chat_design"));
    assert.equal(await design.getAttribute("data-source-topic-id"), "topic_design");

    // A failed save leaves persisted/UI order intact, reports the error, and can be retried.
    const saved = await ids();
    await page.route("**/hosted-device/actions", (route) => route.fulfill({ status: 503, json: { error: "Synthetic move failure" } }), { times: 1 });
    await design.getByRole("button", { name: "Move Design review", exact: true }).dragTo(general.locator(".finite-chat__folder-header"));
    await page.getByRole("alert").getByText("Synthetic move failure", { exact: true }).waitFor();
    assert.deepEqual(await ids(), saved);
    assert(await history.isVisible());
    await page.reload();
    await history.waitFor();
    assert.deepEqual(await ids(), saved);

    // Keyboard alternative includes position and empty topics; cancelling performs no write.
    await page.getByRole("button", { name: "New topic", exact: true }).click();
    await page.getByLabel("Name", { exact: true }).fill("Empty destination");
    await page.getByRole("button", { name: "Create topic", exact: true }).click();
    const empty = page.locator(".finite-chat__folder").filter({ has: page.getByRole("button", { name: "Collapse Empty destination", exact: true }) });
    await empty.waitFor();
    await design.getByRole("button", { name: "Move Design review", exact: true }).focus();
    await page.keyboard.press("Enter");
    await page.getByLabel("Topic", { exact: true }).selectOption({ label: "Empty destination" });
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    assert.deepEqual(await ids(), saved);
    await design.getByRole("button", { name: "Move Design review", exact: true }).focus();
    await page.keyboard.press("Enter");
    await page.getByLabel("Topic", { exact: true }).selectOption({ label: "Empty destination" });
    await page.route("**/hosted-device/actions", (route) => route.fulfill({ status: 503, json: { error: "Synthetic dialog failure" } }), { times: 1 });
    await page.getByRole("dialog").getByRole("button", { name: "Move chat", exact: true }).click();
    await page.getByRole("dialog").getByRole("alert").waitFor();
    assert.deepEqual(await ids(), saved);
    await page.getByRole("dialog").getByRole("button", { name: "Move chat", exact: true }).click();
    await empty.locator('[data-chat-id="chat_design"]').waitFor();
    assert(await history.isVisible());

    // Cancelled/invalid drag must not dispatch a move.
    let moves = 0;
    page.on("request", (request) => { if (request.postData()?.includes('"MoveChat"')) moves++; });
    const handle = design.getByRole("button", { name: "Move Design review", exact: true });
    const box = await handle.boundingBox();
    assert(box);
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + 70, box.y + 25, { steps: 8 });
    await page.keyboard.press("Escape");
    await page.mouse.up();
    assert.equal(moves, 0);
    await handle.dragTo(page.locator(".finite-chat__messages"));
    assert.equal(moves, 0);

    // Another tab's move arrives through the real provider's stream without switching this tab.
    const other = await context.newPage();
    await other.goto(url);
    await other.getByRole("button", { name: "Move Design review", exact: true }).click();
    await other.getByLabel("Topic", { exact: true }).selectOption({ label: "Home" });
    await other.getByLabel("Position", { exact: true }).selectOption({ index: 0 });
    await other.getByRole("dialog").getByRole("button", { name: "Move chat", exact: true }).click();
    await home.locator('[data-chat-id="chat_design"]').waitFor();
    assert.equal((await ids())[0], "chat_design");
    assert(await history.isVisible());
    await other.close();
    await page.screenshot({ path: "/tmp/sidebar-chat-organization-desktop.png", fullPage: true });

    const touchContext = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
    const touch = await touchContext.newPage();
    await touch.goto(url);
    await touch.getByRole("button", { name: "Open chats", exact: true }).first().tap();
    await touch.getByRole("button", { name: "Move Design review", exact: true }).tap();
    await touch.getByLabel("Topic", { exact: true }).selectOption({ label: "General" });
    await touch.screenshot({ path: "/tmp/sidebar-chat-organization-touch-dialog.png", fullPage: true });
    await touch.getByRole("dialog").getByRole("button", { name: "Move chat", exact: true }).tap();
    await touch.getByRole("dialog").waitFor({ state: "hidden" });
    await touch.locator('[data-topic-id="topic_design"] [data-chat-id="chat_design"]').waitFor();
    await touch.screenshot({ path: "/tmp/sidebar-chat-organization-touch.png", fullPage: true });
    await touchContext.close();
    // New dashboard + older hosted service: original chats remain usable,
    // and unsupported movement is not offered.
    await page.route("**/hosted-device/updates?*", (route) => route.abort());
    await page.route("**/hosted-device/state*", async (route) => {
      const response = await route.fetch();
      const legacy = await response.json();
      for (const topic of legacy.topics ?? []) for (const chat of topic.chats) delete chat.placement;
      await route.fulfill({ response, json: legacy });
    });
    await page.reload();
    await history.waitFor();
    assert.equal(await page.locator(".finite-chat__drag-handle").count(), 0);
    assert(await general.getByRole("button", { name: "Design review", exact: true }).isVisible());
    assert.deepEqual(errors, []);
  } catch (error) {
    console.error(output.slice(-8000));
    throw error;
  } finally {
    await browser?.close();
    server.kill("SIGTERM");
    await once(server, "exit");
  }
});
