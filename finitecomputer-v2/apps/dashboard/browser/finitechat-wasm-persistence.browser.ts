import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { chromium, type BrowserContext, type Page, type Request } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

const base = process.env.FINITECHAT_WASM_SPIKE_URL;
const chatUrl = `${base}/dashboard/machines/wasm-hermes/chat`;
const launch = () => ({ ...chromiumLaunchOptions(), headless: true, viewport: { width: 1150, height: 960 } });
const composer = (page: Page) => page.getByRole("textbox", { name: "Message your agent" });
async function send(page: Page, text: string) {
  await composer(page).fill(text, { timeout: 60_000 });
  await page.getByRole("button", { name: "Send message", exact: true }).click();
}
async function reply(page: Page, text: string) {
  await page.getByText(text, { exact: true }).waitFor({ timeout: 90_000 });
}
async function checkpointRows(page: Page) {
  return page.evaluate(async () => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const req = indexedDB.open("finitechat-wasm", 1);
      req.onsuccess = () => resolve(req.result); req.onerror = () => reject(req.error);
    });
    try {
      return await new Promise<{ id: string; revision: number; iv: number[]; ciphertext: number[] }[]>((resolve, reject) => {
        const tx = db.transaction("devices", "readonly");
        const req = tx.objectStore("devices").getAll();
        tx.oncomplete = () => resolve(req.result.map(row => ({ id: row.id, revision: row.revision,
          iv: Array.from(row.iv as Uint8Array), ciphertext: Array.from(new Uint8Array(row.ciphertext)) })));
        tx.onabort = () => reject(tx.error);
      });
    } finally { db.close(); }
  });
}

// All network traffic and inference are real. Faults are injected at browser I/O boundaries,
// without a production failpoint API or mock FiniteChat/Hermes implementation.
test("browser Device survives reload, browser restart, concurrent tabs and uncertain sends", {
  skip: !base, timeout: 480_000,
}, async () => {
  const profile = await mkdtemp(join(tmpdir(), "finitechat-browser-"));
  let context: BrowserContext | undefined;
  let page: Page | undefined;
  const requests: Request[] = [];
  const mutations: string[] = [];
  const badApi: string[] = [];
  const errors: string[] = [];
  const start = async () => {
    const next = await chromium.launchPersistentContext(profile, launch());
    next.on("request", request => {
      const path = new URL(request.url()).pathname;
      if (path === "/events") requests.push(request);
      if (request.method() === "POST" && ["/account-rooms/bootstrap", "/commits", "/spike/enroll"].includes(path)) mutations.push(path);
    });
    next.on("page", page => page.on("pageerror", error => errors.push(error.message)));
    await next.route("**/api/**", async route => {
      if (new URL(route.request().url()).pathname !== "/api/wasm-spike/bootstrap") {
        badApi.push(route.request().url()); await route.abort();
      } else await route.continue();
    });
    return next;
  };
  try {
    context = await start();
    page = await context.newPage();
    await page.goto(chatUrl);
    await composer(page).waitFor({ timeout: 60_000 });
    await page.locator("button.finite-chat__sidebar-new-chat-fab").click();
    await page.locator("button.finite-chat__sidebar-new-chat-fab").evaluate(async button => { while ((button as HTMLButtonElement).disabled) await new Promise(r => setTimeout(r, 25)); });
    const marker = `memory-${crypto.randomUUID()}`;
    await send(page, `Remember ${marker}. Reply with exactly: stored-in-this-chat`);
    await reply(page, "stored-in-this-chat");
    const initialDevice = requests[0].postDataJSON().event.sender.device_id;
    const initialRoom = requests[0].postDataJSON().event.room_id;
    const oldCheckpoint = (await checkpointRows(page))[0];

    await page.reload();
    await reply(page, "stored-in-this-chat");
    await send(page, "What code did I ask you to remember? Reply with only that code.");
    await reply(page, marker);
    console.log("PASS: reload restored the same Device, Room, transcript and Hermes conversation");

    const other = await context.newPage();
    await other.goto(chatUrl);
    await reply(other, marker);
    await Promise.all([
      send(page, "Reply with exactly: first-tab-answer"),
      send(other, "Reply with exactly: second-tab-answer"),
    ]);
    await Promise.all([reply(page, "first-tab-answer"), reply(page, "second-tab-answer"),
      reply(other, "first-tab-answer"), reply(other, "second-tab-answer")]);
    await other.close();
    console.log("PASS: concurrent tabs sent from one Device and converged on both replies");

    // Kill the browser document with a durably prepared send, either before delivery
    // or after real relay acceptance with the receipt withheld from the browser.
    for (const accepted of [false, true]) {
      const answer = accepted ? "accepted-before-crash" : "prepared-before-crash";
      let release!: () => void;
      let witness!: (value: { body: string; receipt?: { seq: number; message_id: string } }) => void;
      const captured = new Promise<{ body: string; receipt?: { seq: number; message_id: string } }>(resolve => { witness = resolve; });
      const held = new Promise<void>(resolve => { release = resolve; });
      await page.route("**/events", async route => {
        const body = route.request().postData()!;
        const response = accepted ? await route.fetch() : undefined;
        if (response) assert.equal(response.status(), 200);
        witness({ body, receipt: await response?.json() });
        await held;
        await route.abort().catch(() => {});
      });
      await send(page, `Reply with exactly: ${answer}`);
      const original = await captured;
      const persisted = await checkpointRows(page);
      assert.equal(persisted.length, 1);
      await page.close(); release();
      page = await context.newPage();
      let replayReceipt: { seq: number; message_id: string } | undefined;
      await page.route("**/events", async route => {
        const response = await route.fetch();
        if (route.request().postData() === original.body) replayReceipt = await response.json();
        await route.fulfill({ response });
      });
      await page.goto(chatUrl);
      await reply(page, answer);
      const attempts = requests.filter(request => request.postData() === original.body);
      assert.ok(attempts.length >= 2, "Restart republishes the exact saved randomized ciphertext");
      assert.ok(replayReceipt, "A real relay receipt completed the saved send");
      if (accepted) assert.deepEqual(replayReceipt, original.receipt, "Relay returns the original acceptance receipt");
      assert.equal(await page.getByText(`Reply with exactly: ${answer}`, { exact: true }).count(), 1);
      assert.equal(await page.getByText(answer, { exact: true }).count(), 1);
      await page.unroute("**/events");
      console.log(`PASS: ${accepted ? "accepted" : "prepared"} send survived tab death, exact ciphertext retry, one transcript entry`);
    }

    // Abort a real IndexedDB write before it can store the consumed MLS generation.
    await page.evaluate(() => {
      const original = IDBObjectStore.prototype.put;
      IDBObjectStore.prototype.put = function () {
        IDBObjectStore.prototype.put = original;
        this.transaction.abort();
        throw new DOMException("Synthetic storage-full failure", "QuotaExceededError");
      };
    });
    const beforeFailure = requests.length;
    await send(page, "Reply with exactly: after-storage-failure");
    await page.getByText(/Checkpoint write failed/).first().waitFor({ timeout: 10_000 });
    assert.equal(requests.length, beforeFailure, "Failed durable preparation publishes nothing");
    await page.reload();
    await send(page, "Reply with exactly: after-storage-failure");
    await reply(page, "after-storage-failure");
    console.log("PASS: a failed IndexedDB write stopped publication and reload recovered a sendable Device");

    const records = await checkpointRows(page);
    assert.equal(records.length, 1);
    assert.ok(records[0].ciphertext.length > 1000);
    assert.equal(new TextDecoder().decode(new Uint8Array(records[0].ciphertext)).includes(marker), false);
    assert.equal(JSON.stringify(records).includes("nsec1"), false);
    await context.close();
    context = await start(); page = await context.newPage();
    await page.goto(chatUrl);
    await reply(page, marker);
    await send(page, "Reply with exactly: survived-browser-restart");
    await reply(page, "survived-browser-restart");
    console.log("PASS: a new browser process reopened the encrypted Device and could still chat");

    assert.deepEqual(new Set(requests.map(request => request.postDataJSON().event.sender.device_id)), new Set([initialDevice]));
    assert.deepEqual(new Set(requests.map(request => request.postDataJSON().event.room_id)), new Set([initialRoom]));
    assert.equal(mutations.filter(path => path === "/account-rooms/bootstrap").length, 0, "No replacement Room on restart");
    assert.equal(mutations.filter(path => path === "/spike/enroll").length, 1, "No replacement Device admission on restart");
    assert.deepEqual(badApi, []); assert.deepEqual(errors, []);
    await page.screenshot({ path: process.env.FINITECHAT_WASM_SCREENSHOT || "/tmp/finitechat-wasm-persistence.png", fullPage: true });

    // Restore an older *authentic* checkpoint on this synthetic profile. The
    // native currency guard must refuse sends rather than reuse an MLS generation.
    await page.evaluate(async record => {
      const db = await new Promise<IDBDatabase>(resolve => { const r = indexedDB.open("finitechat-wasm", 1); r.onsuccess = () => resolve(r.result); });
      const tx = db.transaction("devices", "readwrite");
      tx.objectStore("devices").put({ ...record, iv: new Uint8Array(record.iv), ciphertext: new Uint8Array(record.ciphertext).buffer });
      await new Promise<void>((resolve, reject) => { tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error); }); db.close();
    }, oldCheckpoint);
    const beforeRollback = requests.length;
    await page.reload();
    await page.getByText(/behind.*server|rewound|older than/i).first().waitFor({ timeout: 20_000 });
    assert.equal(requests.length, beforeRollback, "Stale authenticated checkpoint cannot publish");
    assert.equal(mutations.length, 1, "Rollback does not replace the Room or Device");
    console.log("PASS: stale checkpoint refused by native sender-currency protection");
  } catch (error) {
    if (page && !page.isClosed()) {
      await page.screenshot({ path: "/tmp/finitechat-wasm-persistence-failure.png", fullPage: true });
      console.error(await page.locator("main").innerText());
    }
    throw error;
  } finally {
    await context?.close();
    await rm(profile, { recursive: true, force: true });
  }
});
