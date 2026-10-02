import asyncio, json, pathlib, sys
from playwright.async_api import async_playwright
root=pathlib.Path('/tmp/grafana-cleanup-evidence')
fixture=json.loads((root/'fixture-time.json').read_text())
phase=sys.argv[1]
url=f"http://127.0.0.1:13000/d/finite-production-mvp/finite-production-mvp?orgId=1&from={fixture['from']}&to={fixture['to']}&timezone=utc&refresh="
async def main():
 async with async_playwright() as p:
  browser=await p.chromium.launch(executable_path='/usr/bin/chromium',headless=True,args=['--no-sandbox'])
  page=await browser.new_page(viewport={'width':1600,'height':1100},device_scale_factor=1)
  errors=[]
  page.on('pageerror',lambda e:errors.append(str(e)))
  await page.goto(url,wait_until='domcontentloaded')
  await page.get_by_text('Current Production Status',exact=True).first.wait_for(timeout=60000)
  await page.wait_for_timeout(4500)
  await page.screenshot(path=str(root/f'{phase}-synthetic-overview.png'))
  (root/f'{phase}-body.txt').write_text(await page.locator('body').inner_text())
  (root/f'{phase}-dom.json').write_text(json.dumps(await page.locator('[data-testid]').evaluate_all('(els)=>els.map(e=>({testid:e.getAttribute("data-testid"),text:e.textContent?.slice(0,180)}))'),indent=2))
  (root/f'{phase}-browser.json').write_text(json.dumps({'url':page.url,'viewport':{'width':1600,'height':1100},'page_errors':errors,'synthetic':True},indent=2))
  print('Captured',phase,'with',len(errors),'page errors')
  await browser.close()
asyncio.run(main())
