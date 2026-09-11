import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium, type BrowserContext, type Page, type Request } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

const historyTest = process.env.FINITECHAT_WASM_HISTORY_TEST === "1";
const base = process.env.FINITECHAT_WASM_SPIKE_URL;
const url = `${base}/dashboard/machines/wasm-hermes/chat`;
const composer = (page: Page) => page.getByRole("textbox", { name: "Message your agent" });
const transcript = (page: Page) => page.locator(".finite-chat__messages");
async function send(page: Page, text: string) {
  await composer(page).fill(text, { timeout: 60_000 });
  await page.getByRole("button", { name: "Send message", exact: true }).click();
}
async function reply(page: Page, text: string) {
  await transcript(page).getByText(text, { exact: true }).waitFor({ timeout: 100_000 });
}

test("independent browsers join through the agent, restore metadata and share future messages", {
  skip: !base, timeout: 480_000,
}, async () => {
  const profiles = await Promise.all([mkdtemp(join(tmpdir(), "fc-browser-a-")), mkdtemp(join(tmpdir(), "fc-browser-b-"))]);
  let a: BrowserContext | undefined, b: BrowserContext | undefined;
  let pausedPid: number | undefined;
  const sends: Request[][] = [[], []], enrollments: Request[][] = [[], []];
  const forbidden: string[] = [], errors: string[] = [];
  const headers = new Map<Request, Promise<Record<string, string>>>();
  async function start(index: number) {
    const ctx = await chromium.launchPersistentContext(profiles[index], { ...chromiumLaunchOptions(), headless: true, viewport: { width: 1150, height: 960 } });
    ctx.on("request", req => {
      const path = new URL(req.url()).pathname;
      if (req.method() === "POST") headers.set(req, req.allHeaders());
      if (req.method() === "POST" && path === "/events") sends[index].push(req);
      if (req.method() === "POST" && path === "/spike/enroll") enrollments[index].push(req);
      if (/^\/v1\/(hermes|agentd)/.test(path)) forbidden.push(path);
    });
    ctx.on("page", p => p.on("pageerror", e => errors.push(e.message)));
    await ctx.route("**/api/**", async route => {
      const path = new URL(route.request().url()).pathname;
      if (path !== "/api/wasm-spike/bootstrap") { forbidden.push(path); await route.abort(); }
      else await route.continue();
    });
    return ctx;
  }
  const marker = `token-${crypto.randomUUID()}`;
  const otherMarker = `other-${crypto.randomUUID()}`;
  const originalChat = new RegExp(`^The test token is ${marker.slice(0, 15)}`);
  try {
    a = await start(0);
    let pa = await a.newPage(); await pa.goto(url);
    await composer(pa).waitFor({ timeout: 60_000 });
    await pa.locator("button.finite-chat__sidebar-new-chat-fab").click();
    await pa.locator("button.finite-chat__sidebar-new-chat-fab").evaluate(async button => { while ((button as HTMLButtonElement).disabled) await new Promise(r => setTimeout(r, 25)); });
    await send(pa, `The test token is ${marker}. Keep it in this conversation without tools or memory files. Reply with exactly: browser-a-ready`);
    await reply(pa, "browser-a-ready");
    if (historyTest) {
      await send(pa, "Reply with exactly: older-second-reply"); await reply(pa, "older-second-reply");
      await pa.locator("button.finite-chat__sidebar-new-chat-fab").click();
      await pa.locator("button.finite-chat__sidebar-new-chat-fab").evaluate(async button => { while ((button as HTMLButtonElement).disabled) await new Promise(r => setTimeout(r, 25)); });
      await send(pa, `Reply with exactly: ${otherMarker}`); await reply(pa, otherMarker);
      await pa.getByRole("button", { name: originalChat }).first().click();
      await reply(pa, "browser-a-ready");
    }
    await a.close(); a = undefined; // No old human Device online to admit or sync B.

    b = await start(1);
    const pb = await b.newPage(); await pb.goto(url);
    await composer(pb).waitFor({ timeout: 60_000 });
    await pb.getByRole("button", { name: originalChat }).first().click();
    await pb.locator("button.finite-chat__sidebar-new-chat-fab").evaluate(async button => { while ((button as HTMLButtonElement).disabled) await new Promise(r => setTimeout(r, 25)); });
    assert.equal(await transcript(pb).count(), 0, "Fresh browser has chat metadata but no old transcript");
    console.log("PASS: B joined while A's entire browser process was offline; same Chats, no transcript backfill");

    await send(pb, "What is the test token I gave you earlier in this chat? Reply with only that token.");
    await reply(pb, marker); // Same actual Hermes Chat/session.
    assert.equal(await transcript(pb).getByText("browser-a-ready", { exact: true }).count(), 0);
    a = await start(0);
    pa = await a.newPage(); await pa.goto(url);
    await reply(pa, marker);
    await reply(pa, "browser-a-ready");
    await send(pa, "Reply with exactly: future-from-a");
    await Promise.all([reply(pa, "future-from-a"), reply(pb, "future-from-a")]);
    await send(pb, "Reply with exactly: future-from-b");
    await Promise.all([reply(pa, "future-from-b"), reply(pb, "future-from-b")]);
    await pb.reload(); await reply(pb, "future-from-a");
    assert.equal(await transcript(pb).getByText("browser-a-ready", { exact: true }).count(), 0);
    console.log("PASS: independent MLS Devices share future messages and real Hermes context; old Device catches up across Add; reload preserves B");

    if (historyTest) {
      const stateDir = process.env.FINITECHAT_WASM_SPIKE_STATE;
      assert.ok(stateDir, "History fault test requires the disposable harness state directory");
      const service = JSON.parse(await readFile(join(stateDir, "service-ready.json"), "utf8"));
      assert.equal(service.service, "finitechat-hermes");
      assert.equal(service.server_url, "http://127.0.0.1:28789");
      pausedPid = service.pid;
      console.log("History controls before outage:", await pb.getByRole("button", { name: "Load earlier messages from agent", exact: true }).count());
      process.kill(pausedPid!, "SIGSTOP");
      const beforeHistory = sends[1].length;
      await pb.getByRole("button", { name: "Load earlier messages from agent", exact: true }).click({ timeout: 10_000 });
      console.log("History request sent while agent is suspended");
      await pb.getByText("History has not arrived. You can retry; live chat is still available.", { exact: true }).waitFor({ timeout: 25_000 });
      await send(pa, "Reply with exactly: live-during-history-outage");
      await transcript(pb).getByText("Reply with exactly: live-during-history-outage", { exact: true }).waitFor({ timeout: 10_000 });
      await a.close(); a = undefined;
      process.kill(pausedPid!, "SIGCONT"); pausedPid = undefined;
      await reply(pb, "live-during-history-outage");
      console.log("PASS: history timeout did not disable live encrypted messaging; agent resumed and handled the queued request with A offline");
      for (let page = 0; page < 10; page++) {
        await pb.getByRole("button", { name: "Fetching history from agent…", exact: true }).waitFor({ state: "hidden", timeout: 30_000 });
        const more = pb.getByRole("button", { name: "Load earlier messages from agent", exact: true });
        if (!await more.count()) break;
        await more.click();
        await pb.getByRole("button", { name: "Fetching history from agent…", exact: true }).waitFor({ timeout: 10_000 });
      }
      await reply(pb, "browser-a-ready"); await reply(pb, "older-second-reply");
      assert.ok(sends[1].length - beforeHistory >= 2, "More than one encrypted history page was requested");
      assert.equal(await transcript(pb).getByText("browser-a-ready", { exact: true }).count(), 1);
      assert.equal(await transcript(pb).getByText("future-from-a", { exact: true }).count(), 1, "Live tail is not duplicated by history");
      await pb.reload(); await reply(pb, "browser-a-ready");
      const afterReload = sends[1].length;
      await pb.getByRole("button", { name: new RegExp(`^Reply with exactly: ${otherMarker.slice(0, 12)}`) }).first().click();
      await pb.locator("button.finite-chat__sidebar-new-chat-fab").evaluate(async button => { while ((button as HTMLButtonElement).disabled) await new Promise(r => setTimeout(r, 25)); });
      assert.equal(await transcript(pb).count(), 0, "Fetching one Chat did not import another Chat's transcript");
      await pb.getByRole("button", { name: "Load earlier messages from agent", exact: true }).click({ timeout: 10_000 });
      await reply(pb, otherMarker);
      assert.ok(sends[1].length > afterReload, "The other Chat needs its own request to the agent");
      console.log("PASS: paged per-Chat history comes from the agent, persists across reload, leaves the live tail intact, and does not backfill unrelated Chats");
    }

    const ids = sends.map(list => new Set(list.map(r => r.postDataJSON().event.sender.device_id)));
    assert.equal(ids[0].size, 1); assert.equal(ids[1].size, 1);
    assert.notEqual([...ids[0]][0], [...ids[1]][0]);
    assert.equal(new Set(sends.flat().map(r => r.postDataJSON().event.room_id)).size, 1);
    assert.equal(enrollments[0].length, 1); assert.equal(enrollments[1].length, 1);
    const enrollment = enrollments[1][0];
    const unsigned = await pb.request.post(enrollment.url(), { data: enrollment.postData()!, headers: { "Content-Type": "application/json" } });
    assert.equal(unsigned.status(), 401);
    const tampered = await pb.request.post(enrollment.url(), { data: enrollment.postData()!.replace("wasm-hermes-room", "other-room"), headers: {
      "Content-Type": "application/json", Authorization: (await headers.get(enrollment))!.authorization,
    } });
    assert.equal(tampered.status(), 401);
    for (const request of sends.flat()) {
      assert.match((await headers.get(request))!.authorization, /^Nostr /);
      assert.equal(request.postData()!.includes(marker), false);
      assert.equal(Buffer.from(request.postDataJSON().event.envelope.payload, "base64").includes(Buffer.from(marker)), false);
    }
    assert.deepEqual(forbidden, []); assert.deepEqual(errors, []);
    await pb.screenshot({ path: "/tmp/finitechat-multibrowser.png", fullPage: true });
    console.log("PASS: distinct Devices, one Room, signed encrypted relay traffic; admission unsigned/tampered denied; no web bridge");
  } catch (e) {
    if (pausedPid) { process.kill(pausedPid, "SIGCONT"); pausedPid = undefined; }
    for (const ctx of [a,b]) for (const page of (ctx?.pages() ?? []).filter(page => page.url().includes("/dashboard/"))) { console.log(await page.locator("main").innerText({ timeout: 5000 }).catch(() => "")); await page.screenshot({ path: "/tmp/finitechat-history-failure.png" }).catch(() => {}); }
    throw e;
  } finally {
    if (pausedPid) process.kill(pausedPid, "SIGCONT");
    await a?.close(); await b?.close();
    await Promise.all(profiles.map(p => rm(p, { recursive: true, force: true })));
  }
});

test("agent enrollment rejects valid signatures for the wrong account or Room", {
  skip: !base, timeout: 60_000,
}, async () => {
  const browser = await chromium.launch({ ...chromiumLaunchOptions(), headless: true });
  try {
    const page = await browser.newPage(); await page.goto(url);
    await composer(page).waitFor({ timeout: 60_000 });
    const failures = await page.evaluate(async () => {
      const bootstrap = await (await fetch("/api/wasm-spike/bootstrap", { method: "POST" })).json();
      const moduleUrl = "/wasm-spike/finitechat_wasm.js";
      const wasm = await import(moduleUrl);
      await wasm.default({ module_or_path: "/wasm-spike/finitechat_wasm_bg.wasm" });
      let unrelatedSecret = "";
      for (const byte of crypto.getRandomValues(new Uint8Array(32))) unrelatedSecret += byte.toString(16).padStart(2, "0");
      const results: string[] = [];
      for (const variant of ["account", "room"]) {
        const client = new wasm.BrowserChat(variant === "account" ? unrelatedSecret : bootstrap.nsec,
          bootstrap.serverUrl, `denied_${crypto.randomUUID()}`, undefined, () => Promise.resolve());
        try {
          await client.join(bootstrap.agentAccountId, bootstrap.agentUrl, variant === "room" ? "wrong-room" : bootstrap.room);
          results.push("unexpected admission");
        } catch (error) { results.push(String(error)); }
        finally { client.free(); }
      }
      return results;
    });
    assert.equal(failures.length, 2);
    for (const failure of failures) assert.match(failure, /spike\/enroll: HTTP 403/);
    console.log("PASS: correctly signed enrollment still requires the configured account and canonical Room");
  } finally { await browser.close(); }
});
