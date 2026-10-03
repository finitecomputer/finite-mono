import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import http from "node:http";
import { test } from "node:test";
import { chromium, type Browser, type Locator } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

test("sidebar direct drag, saved topic order, failures, keyboard movement, and touch dragging", { timeout: 180_000 }, async () => {
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
    const drag = async (row: Locator, target: Locator) => {
      await row.and(page.locator('[draggable="true"]')).waitFor();
      await row.locator(".finite-chat__thread-open").dragTo(target);
    };
    assert.equal(await page.locator(".finite-chat__drag-handle").count(), 0);
    assert.equal(await page.getByRole("button", { name: /^Move / }).count(), 0);
    const initial = await ids();
    const first = home.locator(`[data-chat-id="${initial[0]}"]`);
    const second = home.locator(`[data-chat-id="${initial[1]}"]`);
    await drag(second, first);
    await page.waitForFunction((expected) => Array.from(document.querySelectorAll('[data-topic-id="home"] .finite-chat__thread-row')).map((row) => row.getAttribute("data-chat-id")).join() === expected, [...initial].reverse().join());
    assert(await history.isVisible(), "background reordering must retain the active transcript");

    // Cross-topic drag into collapsed Home expands it; reload retains the selected chat.
    await page.getByRole("button", { name: "Collapse Home", exact: true }).click();
    await drag(design, home.locator(".finite-chat__folder-header"));
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
    await drag(design, general.locator(".finite-chat__folder-header"));
    await page.getByRole("alert").getByText("Synthetic move failure", { exact: true }).waitFor();
    assert.deepEqual(await ids(), saved);
    assert(await history.isVisible());
    await page.reload();
    await history.waitFor();
    assert.deepEqual(await ids(), saved);

    // Original click, rename/archive actions and context-menu behavior stay intact.
    await design.locator(".finite-chat__thread-open").click();
    await design.hover();
    await design.getByRole("button", { name: "Rename Design review", exact: true }).click();
    await page.getByRole("dialog", { name: "Rename chat" }).waitFor();
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await design.hover();
    await design.getByRole("button", { name: "Archive Design review", exact: true }).click();
    await design.getByRole("button", { name: "Restore Design review", exact: true }).waitFor();
    await design.hover();
    await design.getByRole("button", { name: "Restore Design review", exact: true }).click();
    await design.getByRole("button", { name: "Archive Design review", exact: true }).waitFor();
    assert.deepEqual(await ids(), saved);
    await page.evaluate(() => document.addEventListener("contextmenu", (event) => {
      document.body.dataset.contextMenuPrevented = String(event.defaultPrevented);
    }, { once: true }));
    await design.locator(".finite-chat__thread-open").click({ button: "right" });
    assert.equal(await page.locator("body").getAttribute("data-context-menu-prevented"), "false");
    await page.keyboard.press("Escape");

    // Keyboard movement operates on the original chat button, with no extra UI.
    await page.getByRole("button", { name: "New topic", exact: true }).click();
    await page.getByLabel("Name", { exact: true }).fill("Empty destination");
    await page.getByRole("button", { name: "Create topic", exact: true }).click();
    const empty = page.locator(".finite-chat__folder").filter({ has: page.getByRole("button", { name: "Collapse Empty destination", exact: true }) });
    await empty.waitFor();
    const keyboardMove = async (key: string) => {
      await design.and(page.locator('[draggable="true"]')).waitFor();
      await design.locator(".finite-chat__thread-open").focus();
      await page.keyboard.press(`Alt+Shift+${key}`);
    };
    await keyboardMove("ArrowUp");
    await page.waitForFunction(() => document.querySelector('[data-topic-id="home"] .finite-chat__thread-row:nth-child(2)')?.getAttribute("data-chat-id") === "chat_design");
    await keyboardMove("ArrowDown");
    await page.waitForFunction((expected) => Array.from(document.querySelectorAll('[data-topic-id="home"] .finite-chat__thread-row')).map((row) => row.getAttribute("data-chat-id")).join() === expected, saved.join());
    await page.route("**/hosted-device/actions", (route) => route.fulfill({ status: 503, json: { error: "Synthetic keyboard failure" } }), { times: 1 });
    await keyboardMove("ArrowRight");
    await page.getByRole("alert").getByText("Synthetic keyboard failure", { exact: true }).waitFor();
    assert.deepEqual(await ids(), saved);
    await keyboardMove("ArrowRight");
    await general.locator('[data-chat-id="chat_design"]').waitFor();
    await page.waitForFunction(() => document.activeElement?.closest('[data-topic-id]')?.getAttribute("data-topic-id") === "topic_design");
    // A delayed successful save must not take focus back from the composer.
    let releaseSave!: () => void;
    const saveGate = new Promise<void>((resolve) => { releaseSave = resolve; });
    await page.route("**/hosted-device/actions", async (route) => { await saveGate; await route.continue(); }, { times: 1 });
    const pendingSave = page.waitForRequest((request) => request.postData()?.includes('"MoveChat"') ?? false);
    await keyboardMove("ArrowRight");
    await pendingSave;
    await page.locator("textarea").focus();
    releaseSave();
    await empty.locator('[data-chat-id="chat_design"]').waitFor();
    await design.and(page.locator('[draggable="true"]')).waitFor();
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    assert.equal(await page.locator("textarea").evaluate((element) => element === document.activeElement), true);
    assert(await history.isVisible());

    // Cancelled/invalid drag must not dispatch a move.
    let moves = 0;
    page.on("request", (request) => { if (request.postData()?.includes('"MoveChat"')) moves++; });
    const handle = design.locator(".finite-chat__thread-open");
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
    const otherDesign = other.locator('[data-chat-id="chat_design"][draggable="true"]');
    await otherDesign.waitFor();
    await otherDesign.locator(".finite-chat__thread-open").dragTo(other.locator('[data-topic-id="home"] .finite-chat__thread-row').first());
    await home.locator('[data-chat-id="chat_design"]').waitFor();
    assert.equal((await ids())[0], "chat_design");
    assert(await history.isVisible());
    await other.close();
    await page.screenshot({ path: "/tmp/sidebar-direct-drag-desktop-reorganized.png", fullPage: true });

    const touchContext = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
    const touch = await touchContext.newPage();
    await touch.goto(url);
    await touch.getByRole("button", { name: "Open chats", exact: true }).first().tap();
    const touchDesign = touch.locator('[data-chat-id="chat_design"][draggable="true"]');
    await touchDesign.waitFor();
    // A normal tap still opens the chat and closes mobile navigation.
    await touchDesign.locator(".finite-chat__thread-open").tap();
    await touch.getByRole("button", { name: "Open chats", exact: true }).tap();
    const cdp = await touchContext.newCDPSession(touch);
    const touchPoint = async (locator: Locator) => {
      // Raw CDP touches bypass Playwright's actionability checks. Wait for the
      // sidebar's slide-in animation to settle before sampling coordinates.
      await locator.tap({ trial: true });
      const box = await locator.boundingBox();
      assert(box);
      const point = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
      assert(await locator.evaluate((element, point) => element.contains(document.elementFromPoint(point.x, point.y)), point), "touch coordinates must hit the intended element");
      return point;
    };
    let point = await touchPoint(touchDesign.locator(".finite-chat__thread-open"));
    let touchMoves = 0;
    touch.on("request", (request) => { if (request.postData()?.includes('"MoveChat"')) touchMoves++; });
    // Moving before the hold behaves as a swipe, never a move.
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ ...point, y: point.y + 25 }] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await touch.waitForTimeout(400);
    assert.equal(touchMoves, 0);
    // A cancelled long-press drag is also a no-op.
    point = await touchPoint(touchDesign.locator(".finite-chat__thread-open"));
    const destination = await touchPoint(touch.locator('[data-topic-id="topic_design"] .finite-chat__folder-header'));
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point] });
    await touchDesign.and(touch.locator('[data-dragging="true"]')).waitFor();
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [destination] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchCancel", touchPoints: [] });
    assert.equal(touchMoves, 0);
    // Long-press and drag the original row directly to General.
    point = await touchPoint(touchDesign.locator(".finite-chat__thread-open"));
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point] });
    await touchDesign.and(touch.locator('[data-dragging="true"]')).waitFor();
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [destination] });
    await touch.screenshot({ path: "/tmp/sidebar-direct-drag-touch-active.png", fullPage: true });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await touch.locator('[data-topic-id="topic_design"] [data-chat-id="chat_design"]').waitFor();
    assert.equal(touchMoves, 1);
    assert.equal(await touch.getByRole("dialog").count(), 0);
    await touch.screenshot({ path: "/tmp/sidebar-direct-drag-touch-reorganized.png", fullPage: true });
    await touch.reload();
    await touch.getByText("I kept this conversation after the local dashboard restarted.", { exact: true }).waitFor();
    await touch.getByRole("button", { name: "Open chats", exact: true }).tap();
    await touch.locator('[data-topic-id="topic_design"] [data-chat-id="chat_design"]').waitFor();

    // Failed touch saves preserve the same row and transcript after reload.
    const touchSource = await touchPoint(touchDesign.locator(".finite-chat__thread-open"));
    const touchHome = await touchPoint(touch.locator('[data-topic-id="home"] .finite-chat__folder-header'));
    await touch.route("**/hosted-device/actions", (route) => route.fulfill({ status: 503, json: { error: "Synthetic touch failure" } }), { times: 1 });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [touchSource] });
    await touchDesign.and(touch.locator('[data-dragging="true"]')).waitFor();
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [touchHome] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await touch.getByRole("alert").getByText("Synthetic touch failure", { exact: true }).waitFor();
    await touch.reload();
    await touch.getByRole("button", { name: "Open chats", exact: true }).tap();
    await touch.locator('[data-topic-id="topic_design"] [data-chat-id="chat_design"]').waitFor();

    // Holding at the sidebar edge reaches offscreen topics without lifting the finger.
    for (let index = 0; index < 20; index++) {
      const response = await touch.request.post(`${api}/actions`, { data: { CreateTopic: { room_id: "room_design", title: `Scroll destination ${index}` } } });
      assert(response.ok());
    }
    const farFolder = touch.locator(".finite-chat__folder").filter({ has: touch.getByRole("button", { name: "Collapse Scroll destination 19", exact: true }) });
    await farFolder.waitFor();
    const farId = await farFolder.getAttribute("data-topic-id");
    const nav = touch.locator(".finite-chat__sidebar-nav");
    const scrollSource = await touchPoint(touchDesign.locator(".finite-chat__thread-open"));
    const navBounds = await nav.boundingBox();
    assert(navBounds);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [scrollSource] });
    await touchDesign.and(touch.locator('[data-dragging="true"]')).waitFor();
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: navBounds.x + 80, y: navBounds.y + navBounds.height - 8 }] });
    await touch.waitForFunction((topicId) => {
      const nav = document.querySelector(".finite-chat__sidebar-nav");
      const header = document.querySelector(`[data-topic-id="${topicId}"] .finite-chat__folder-header`);
      return nav && header && nav.scrollTop > 100 && header.getBoundingClientRect().bottom <= nav.getBoundingClientRect().bottom;
    }, farId);
    const farBounds = await farFolder.locator(".finite-chat__folder-header").boundingBox();
    assert(farBounds);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: farBounds.x + 80, y: farBounds.y + farBounds.height / 2 }] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await farFolder.locator('[data-chat-id="chat_design"]').waitFor();
    assert.equal(touchMoves, 3);
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
    assert.equal(await page.locator('[data-chat-id][draggable="true"]').count(), 0);
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
