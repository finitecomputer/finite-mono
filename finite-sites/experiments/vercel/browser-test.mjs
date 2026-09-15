import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { state } from './operator.mjs';

// Use an existing browser; never install system dependencies for this test.
const browser = await chromium.launch({ executablePath: process.env.POC_BROWSER_PATH || chromium.executablePath(), headless: true });
try {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto('http://127.0.0.1:4319');
  await page.getByRole('button', { name: 'Grant access', exact: true }).click();
  await page.waitForURL('http://127.0.0.1:4319/');
  const popupPromise = context.waitForEvent('page');
  await page.getByRole('button', { name: 'Open private site', exact: true }).click();
  const site = await popupPromise;
  await site.waitForURL('https://finite-sites-poc-alpha.vercel.app/', { timeout: 30000 });
  await site.getByRole('heading', { name: 'Content version 2' }).waitFor();
  assert.equal(await site.locator('body').evaluate(element => getComputedStyle(element).backgroundColor), 'rgb(244, 245, 239)');
  await site.screenshot({ path: join(state, 'browser-authorized.png'), fullPage: true });
  await page.getByRole('button', { name: 'Revoke access', exact: true }).click();
  await page.waitForURL('http://127.0.0.1:4319/');
  const denied = await site.reload();
  assert.equal(denied.status(), 403);
  assert.equal((await site.goto('https://finite-sites-poc-alpha.vercel.app/assets/private.txt')).status(), 403);
  await site.screenshot({ path: join(state, 'browser-revoked.png') });
  const other = await context.newPage();
  assert.equal((await other.goto('https://finite-sites-poc-beta.vercel.app/')).status(), 403);
  console.log('Browser: real sign-in handoff, styled private page, revoke, denied reload/asset, and second-site denial passed.');
} finally { await browser.close(); }
