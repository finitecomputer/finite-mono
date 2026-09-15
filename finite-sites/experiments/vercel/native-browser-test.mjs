import {chromium} from 'playwright';import assert from 'node:assert/strict';import {writeFile} from 'node:fs/promises';import {root} from './operator.mjs';import {nativeShare} from './native-client.mjs';
const browser=await chromium.launch({executablePath:process.env.POC_BROWSER_PATH||chromium.executablePath(),headless:true});
try{
 const context=await browser.newContext(),page=await context.newPage();await page.goto('http://127.0.0.1:4319');const panel=page.locator('[data-site="gamma"]');
 await panel.getByRole('button',{name:'Grant native viewer'}).click();await page.waitForURL('http://127.0.0.1:4319/');
 const popup=context.waitForEvent('page');await panel.getByRole('button',{name:'Open with Finite identity'}).click();const site=await popup;await site.waitForURL('https://finite-sites-poc-native.vercel.app/');await site.getByRole('heading',{name:'Managed Git version 3'}).waitFor();
 assert.equal(await site.locator('body').evaluate(e=>getComputedStyle(e).backgroundColor),'rgb(244, 245, 239)');
 await page.screenshot({path:`${root}/evidence/fidelity-console.png`,fullPage:true});await site.screenshot({path:`${root}/evidence/fidelity-native-viewer.png`,fullPage:true});
 await panel.getByRole('button',{name:'Revoke native viewer'}).click();await page.waitForURL('http://127.0.0.1:4319/');assert.equal((await site.reload()).status(),403);assert.equal((await site.goto('https://finite-sites-poc-native.vercel.app/assets/style.css')).status(),403);
 await writeFile(`${root}/evidence/fidelity-browser.json`,JSON.stringify({at:new Date().toISOString(),checks:['native Finite identity handoff','real browser host-only session','private HTML and CSS','revocation blocks reload and asset']},null,2));console.log('Native browser flow passed.');
}finally{await browser.close();await nativeShare(false);}
