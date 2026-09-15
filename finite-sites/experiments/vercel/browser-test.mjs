import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { readFile,writeFile } from 'node:fs/promises';
import { root,call } from './operator.mjs';
const config=JSON.parse(await readFile(join(root,'config.json')));
const browser=await chromium.launch({executablePath:process.env.POC_BROWSER_PATH||chromium.executablePath(),headless:true});
const email='viewer@example.invalid';
try{
 const context=await browser.newContext();const page=await context.newPage();
 await page.goto('http://127.0.0.1:4319');
 const alphaPanel=page.locator('[data-site="wild-alpha"]'),betaPanel=page.locator('[data-site="wild-beta"]');
 const openSite=async(panel,site)=>{
  await panel.getByRole('button',{name:'Grant access',exact:true}).click();await page.waitForURL('http://127.0.0.1:4319/');
  const popup=context.waitForEvent('page');await panel.getByRole('button',{name:'Open private site',exact:true}).click();
  const tab=await popup;await tab.waitForURL(`https://${site}.${config.siteBaseDomain}/`,{timeout:30000});return tab;
 };
 const alpha=await openSite(alphaPanel,'wild-alpha');
 await alpha.getByRole('heading',{name:'Content version 2'}).waitFor();
 assert.equal(await alpha.locator('body').evaluate(e=>getComputedStyle(e).backgroundColor),'rgb(244, 245, 239)');
 await alpha.evaluate(()=>localStorage.setItem('tenant-marker','wild-alpha'));
 const beta=await openSite(betaPanel,'wild-beta');
 await beta.getByRole('heading',{name:'Content version 2'}).waitFor();
 assert.equal(await beta.evaluate(()=>localStorage.getItem('tenant-marker')),null);
 assert.equal(await alpha.evaluate(async url=>{try{await fetch(url,{credentials:'include'});return false;}catch{return true;}},beta.url()),true);
 const cookies=await context.cookies();
 const sessions=cookies.filter(c=>c.name==='__Host-finite_poc');
 assert.equal(sessions.length,2);assert.equal(new Set(sessions.map(c=>c.domain)).size,2);assert.ok(sessions.every(c=>!c.domain.startsWith('.')&&c.secure&&c.httpOnly));
 await alphaPanel.getByRole('button',{name:'Use version 1',exact:true}).click();await page.waitForURL('http://127.0.0.1:4319/');
 await alpha.reload();await alpha.getByRole('heading',{name:'Content version 1'}).waitFor();
 await beta.reload();await beta.getByRole('heading',{name:'Content version 2'}).waitFor();
 await alphaPanel.getByRole('button',{name:'Use version 2',exact:true}).click();await page.waitForURL('http://127.0.0.1:4319/');
 await alpha.reload();await alpha.getByRole('heading',{name:'Content version 2'}).waitFor();
 await page.screenshot({path:join(root,'evidence/wildcard-console.png'),fullPage:true});
 await alpha.screenshot({path:join(root,'evidence/wildcard-authorized.png'),fullPage:true});
 await alphaPanel.getByRole('button',{name:'Revoke access',exact:true}).click();await page.waitForURL('http://127.0.0.1:4319/');
 assert.equal((await alpha.reload()).status(),403);
 assert.equal((await alpha.goto(`https://wild-alpha.${config.siteBaseDomain}/assets/private.txt`)).status(),403);
 assert.equal((await beta.reload()).status(),200);
 await alpha.screenshot({path:join(root,'evidence/wildcard-revoked.png')});
 await writeFile(join(root,'evidence/wildcard-browser.json'),JSON.stringify({at:new Date().toISOString(),passed:['two real login handoffs','private CSS rendered','separate browser storage','cross-origin response unreadable','host-only Secure HttpOnly cookies','Alpha rollback + restore','Alpha revocation blocks page and asset','Beta unaffected by Alpha revocation']},null,2));
 console.log('Browser proof passed: login, content rollback, tenant isolation and independent revocation.');
}finally{
 await browser.close();
 for(const site of ['wild-alpha','wild-beta'])await call({op:'grant.set',site,email,allowed:false});
}
