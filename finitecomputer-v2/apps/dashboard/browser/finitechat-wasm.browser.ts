import assert from "node:assert/strict";
import { test } from "node:test";
import { chromium, type Request } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

// Opt in against the real Hermes harness. This test uses the configured Finite Private key.
test("real dashboard chats with real Hermes through browser-owned MLS, without a web bridge", {
  skip: !process.env.FINITECHAT_WASM_SPIKE_URL, timeout: 360_000,
}, async () => {
  const base = process.env.FINITECHAT_WASM_SPIKE_URL!;
  const browser = await chromium.launch({ ...chromiumLaunchOptions(), headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1150, height: 960 } });
    const eventRequests: Request[] = [];
    const badApiRequests: string[] = [];
    const pageErrors: string[] = [];
    let wasmLoaded = false;
    page.on("pageerror", error => pageErrors.push(error.message));
    page.on("response", response => {
      if (response.url().endsWith("finitechat_wasm_bg.wasm") && response.ok()) wasmLoaded = true;
    });
    page.on("request", request => {
      if (new URL(request.url()).pathname === "/events") eventRequests.push(request);
    });
    await page.route("**/api/**", async route => {
      const path = new URL(route.request().url()).pathname;
      if (path !== "/api/wasm-spike/bootstrap") {
        badApiRequests.push(path);
        await route.abort();
      } else await route.continue();
    });
    await page.goto(`${base}/dashboard/machines/wasm-hermes/chat`);
    const composer = page.getByRole("textbox", { name: "Message your agent" });
    const marker = `remember-${crypto.randomUUID()}`;
    try {
      await composer.waitFor({ timeout: 60_000 });
      await page.locator("button.finite-chat__sidebar-new-chat-fab").click();
      await page.locator("button.finite-chat__sidebar-new-chat-fab").evaluate(async button => { while ((button as HTMLButtonElement).disabled) await new Promise(r => setTimeout(r, 25)); });
      await composer.fill(`Use your terminal tool to calculate 137 + 286. Reply with just the number. Also remember the code ${marker} for my next message; do not repeat the code yet.`, { timeout: 60_000 });
      await page.getByRole("button", { name: "Send message", exact: true }).click();
      await page.getByText("423", { exact: true }).waitFor({ timeout: 100_000 });
      await composer.fill("What code did I ask you to remember? Reply with only the code.");
      await page.getByRole("button", { name: "Send message", exact: true }).click();
      await page.getByText(marker, { exact: true }).waitFor({ timeout: 100_000 });
      await page.locator("button.finite-chat__sidebar-new-chat-fab").click();
      await page.waitForFunction(() => !document.querySelector(".finite-chat__messages")?.textContent?.includes("What code did I ask"));
      await composer.fill("Reply with exactly: fresh-chat-ready");
      await page.getByRole("button", { name: "Send message", exact: true }).click();
      await page.getByText("fresh-chat-ready", { exact: true }).waitFor({ timeout: 100_000 });
      await page.getByRole("button", { name: /^Use your terminal tool to calculate/ }).first().click();
      await page.getByText(marker, { exact: true }).waitFor({ timeout: 10_000 });
    } catch (error) {
      await page.screenshot({ path: process.env.FINITECHAT_WASM_SCREENSHOT || "/tmp/finitechat-wasm-proof.png", fullPage: true });
      throw new Error(`${error}\n${await page.locator("main").innerText()}\n${pageErrors.join("\n")}`);
    }
    assert.equal(wasmLoaded, true, "A browser loaded the compiled WASM module");
    assert.deepEqual(pageErrors, []);
    assert.deepEqual(badApiRequests, [], "No dashboard chat/hosted-device API was called");
    assert.ok(eventRequests.length >= 4, "Room organization and user messages travel over MLS");
    for (const request of eventRequests) {
      assert.notEqual(new URL(request.url()).origin, new URL(base).origin, "Browser POSTs directly to the relay");
      const body = request.postDataJSON();
      assert.equal(body.event.envelope.kind, "application");
      const cipher = Buffer.from(body.event.envelope.payload, "base64");
      assert.ok(cipher.length > marker.length);
      assert.equal(cipher.includes(Buffer.from(marker)), false, "MLS payload cannot contain the plaintext marker");
      assert.equal(request.postData()!.includes(marker), false);
      assert.match((await request.allHeaders()).authorization, /^Nostr /);
    }
    const sent = eventRequests[0];
    const unsigned = await page.request.post(sent.url(), { data: sent.postData()!, headers: { "Content-Type": "application/json" } });
    assert.equal(unsigned.status(), 401, "Server requires the browser's Nostr signature");
    const tampered = sent.postDataJSON();
    const tamperedBytes = Buffer.from(tampered.event.envelope.payload, "base64");
    tamperedBytes[0] ^= 1;
    tampered.event.envelope.payload = tamperedBytes.toString("base64");
    const changed = await page.request.post(sent.url(), { data: JSON.stringify(tampered), headers: {
      "Content-Type": "application/json", Authorization: (await sent.allHeaders()).authorization,
    } });
    assert.equal(changed.status(), 401, "Signature binds the exact ciphertext request body");
    await page.screenshot({ path: process.env.FINITECHAT_WASM_SCREENSHOT || "/tmp/finitechat-wasm-proof.png", fullPage: true });
    console.log("PASS: real dashboard/Hermes round trips with context; direct signed relay requests; no plaintext on wire; no web bridge; unsigned/tampered requests rejected");
  } finally { await browser.close(); }
});
