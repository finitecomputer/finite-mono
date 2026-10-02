import asyncio,json,pathlib
from playwright.async_api import async_playwright
root=pathlib.Path('/tmp/grafana-cleanup-evidence'); t=json.loads((root/'fixture-time.json').read_text())
async def main():
 async with async_playwright() as p:
  b=await p.chromium.launch(executable_path='/usr/bin/chromium',headless=True,args=['--no-sandbox'])
  page=await b.new_page(viewport={'width':1600,'height':1100},device_scale_factor=1)
  await page.goto(f"http://127.0.0.1:13000/d/finite-production-mvp/finite-production-mvp?from={t['from']}&to={t['to']}&timezone=utc&viewPanel=2",wait_until='domcontentloaded')
  await page.get_by_test_id('data-testid Panel header Public Uptime - 24 Hours').wait_for(timeout=30000)
  await page.wait_for_timeout(2500)
  await page.screenshot(path=str(root/'after-synthetic-uptime-detail.png'))
  await b.close()
asyncio.run(main())
