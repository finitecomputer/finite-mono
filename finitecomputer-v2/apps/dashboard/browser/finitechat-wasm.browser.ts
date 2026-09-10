import assert from "node:assert/strict";
import { test } from "node:test";
import { chromium, type Request } from "playwright";
import { chromiumLaunchOptions } from "../scripts/playwright-browser";

// Opt in against scripts/finitechat-wasm-spike up. No hosted web Device, Core,
// inference API or external account is required by this transport proof.
test("Rust WASM exchanges encrypted FiniteChat with a native agent without a web bridge", {
  skip: !process.env.FINITECHAT_WASM_SPIKE_URL, timeout: 90_000,
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
    await page.goto(`${base}/wasm-spike`);
    await page.getByRole("button", { name: "Sign in as local spike user" }).click();
    await page.getByLabel("Message", { exact: true }).waitFor({ timeout: 45_000 }).catch(async error => {
      throw new Error(`${error}\n${await page.locator("main").innerText()}\n${pageErrors.join("\n")}`);
    });
    const marker = `browser-MLS-proof-${crypto.randomUUID()}`;
    for (const text of [marker, `second-${marker}`]) {
      await page.getByLabel("Message", { exact: true }).fill(text);
      await page.getByRole("button", { name: "Send", exact: true }).click();
      await page.getByText(`Native agent received over MLS: ${text}`, { exact: true }).waitFor({ timeout: 20_000 }).catch(async error => {
        throw new Error(`${error}\n${await page.locator("main").innerText()}\n${pageErrors.join("\n")}`);
      });
    }
    assert.equal(wasmLoaded, true, "A browser loaded the compiled WASM module");
    assert.deepEqual(pageErrors, []);
    assert.deepEqual(badApiRequests, [], "No dashboard chat/hosted-device API was called");
    assert.equal(eventRequests.length, 2);
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
    console.log("PASS: two browser/native MLS round trips; direct signed relay requests; no plaintext on wire; no web bridge; unsigned/tampered requests rejected");
  } finally { await browser.close(); }
});
