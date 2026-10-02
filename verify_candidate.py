import asyncio,json,pathlib
from playwright.async_api import async_playwright
root=pathlib.Path('/tmp/grafana-cleanup-evidence')
time=json.loads((root/'fixture-time.json').read_text())
dashboard=json.loads((root/'candidate.json').read_text())
url=f"http://127.0.0.1:13000/d/finite-production-mvp/finite-production-mvp?orgId=1&from={time['from']}&to={time['to']}&timezone=utc&refresh="
async def main():
 async with async_playwright() as p:
  browser=await p.chromium.launch(executable_path='/usr/bin/chromium',headless=True,args=['--no-sandbox'])
  page=await browser.new_page(viewport={'width':1600,'height':1100},device_scale_factor=1)
  errors=[]; query_errors=[]; checks=[]
  page.on('pageerror',lambda e:errors.append(str(e)))
  async def response(res):
   if '/api/ds/query' in res.url:
    if res.status!=200: query_errors.append({'url':res.url,'status':res.status,'body':await res.text()})
    else:
     try:
      data=await res.json()
      for k,v in data.get('results',{}).items():
       if v.get('error'): query_errors.append({'refId':k,'error':v['error']})
     except Exception: pass
  page.on('response',response)
  async def reveal(title):
   panel=page.get_by_test_id('data-testid Panel header '+title)
   for attempt in range(30):
    if await panel.count():
     await panel.scroll_into_view_if_needed()
     await page.wait_for_timeout(700)
     return panel
    await page.mouse.move(1100,650)
    await page.mouse.wheel(0,600)
    await page.wait_for_timeout(250)
   raise AssertionError('Panel did not mount: '+title)
  await page.goto(url,wait_until='domcontentloaded')
  await page.get_by_text('Current Production Status',exact=True).first.wait_for(timeout=30000)
  await page.wait_for_timeout(2000)
  # Scroll through visible panels to force Grafana's lazy rendering before evidence capture.
  for title in ['LAT Filesystem Used','LAT Network Errors','LAT Recent Warning Logs']:
   panel=await reveal(title)
   await page.wait_for_timeout(1000)
   checks.append({'visible_panel':title,'rendered':await panel.count()==1,'text':(await panel.inner_text())[:350]})
  await page.get_by_test_id('data-testid dashboard-row-toggle-for-Public availability · 7-day history').scroll_into_view_if_needed()
  await page.screenshot(path=str(root/'after-synthetic-collapsed-rows.png'))
  for row in dashboard['panels']:
   if row.get('type')!='row' or not row.get('collapsed'): continue
   toggle=page.get_by_test_id('data-testid dashboard-row-toggle-for-'+row['title'])
   await toggle.scroll_into_view_if_needed()
   # Original child panels must be absent before expanding the row.
   absent_before=all([await page.get_by_test_id('data-testid Panel header '+child['title']).count()==0 for child in row['panels']])
   await toggle.click()
   children=[]
   for child in row['panels']:
    panel=await reveal(child['title'])
    await page.wait_for_timeout(700)
    children.append({'id':child['id'],'title':child['title'],'visible':await panel.is_visible(),'text':(await panel.inner_text())[:400]})
   checks.append({'row_id':row['id'],'title':row['title'],'children_absent_before_expansion':absent_before,'children':children})
   await toggle.scroll_into_view_if_needed()
   await page.screenshot(path=str(root/f'after-synthetic-expanded-row-{row["id"]}.png'))
  for panel_id,title in [(30,'Sites Created per Day — 90 Days (UTC)'),(21,'LAT Kernel Warning Logs')]:
   await page.goto(url+'&viewPanel='+str(panel_id),wait_until='domcontentloaded')
   panel=page.get_by_test_id('data-testid Panel header '+title)
   await panel.wait_for(timeout=30000); await page.wait_for_timeout(2500)
   checks.append({'deep_link':panel_id,'title':title,'visible':await panel.is_visible(),'url':page.url,'text':(await panel.inner_text())[:1000]})
   await page.screenshot(path=str(root/f'after-synthetic-deeplink-{panel_id}.png'))
  (root/'candidate-browser-checks.json').write_text(json.dumps({'synthetic':True,'checks':checks,'page_errors':errors,'query_errors':query_errors},indent=2))
  print(json.dumps({'checks':len(checks),'page_errors':errors,'query_errors':query_errors},indent=2))
  await browser.close()
asyncio.run(main())
