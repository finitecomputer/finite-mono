import assert from "node:assert/strict";
import { test } from "node:test";
import { startTrialAccessMonitor } from "./trial-access-monitor";

test("trial access polling follows transitions and fences failed or obsolete responses", async (t) => {
  t.mock.timers.enable({ apis: ["setInterval"] });
  const refreshes: string[] = [];
  let reply: () => Promise<Response> = async () => Response.json({ blocked: true });
  const requests: RequestInit[] = [];
  t.mock.method(globalThis, "fetch", async (url: string, options: RequestInit) => {
    assert.equal(url, "/api/billing/trial-access");
    requests.push(options);
    return reply();
  });
  const start = (blocked = true, pathname = "/dashboard") => startTrialAccessMonitor({
    blocked, pathname,
    refresh: () => refreshes.push("refresh"),
    replace: (url) => refreshes.push(url),
  });
  const settle = () => new Promise<void>((resolve) => setImmediate(resolve));
  const tick = async () => { t.mock.timers.tick(30_000); await settle(); };
  let stop = start();
  await settle();
  await tick();
  assert.deepEqual(refreshes, [], "unchanged blocked state does not refresh");
  assert.equal(requests[0].cache, "no-store");

  for (const invalid of [null, {}, { blocked: "false" }, { blocked: 0 }]) {
    reply = async () => Response.json(invalid);
    await tick();
  }
  reply = async () => new Response("unavailable", { status: 503 });
  await tick();
  reply = async () => { throw new Error("network unavailable"); };
  await tick();
  reply = async () => new Response("invalid json");
  await tick();
  assert.deepEqual(refreshes, [], "failures never imply recovery");

  reply = async () => Response.json({ blocked: false });
  await tick(); await tick(); await tick(); await tick();
  assert.deepEqual(refreshes, ["refresh", "refresh", "refresh"], "stale/failed renders get only three attempts per transition");
  reply = async () => Response.json({ blocked: true });
  await tick(); await tick();
  reply = async () => Response.json({ blocked: false });
  await tick();
  assert.equal(refreshes.length, 5, "renewed blocking rearms recovery");
  stop();

  let finish!: (response: Response) => void;
  reply = () => new Promise((resolve) => { finish = resolve; });
  stop = start();
  const pendingCount = requests.length;
  await tick(); await tick();
  assert.equal(requests.length, pendingCount, "slow requests do not overlap");
  const oldSignal = requests.at(-1)!.signal!;
  stop();
  assert.equal(oldSignal.aborted, true);
  finish(Response.json({ blocked: false }));
  await settle(); await tick();
  assert.equal(refreshes.length, 5, "unmount suppresses late fetch completion and clears the timer");
  assert.equal(requests.length, pendingCount);

  let finishBody!: (value: unknown) => void;
  reply = async () => ({ ok: true, json: () => new Promise((resolve) => { finishBody = resolve; }) }) as Response;
  stop = start(false);
  await settle();
  stop(); // Effect cleanup on pathname or server prop change.
  reply = async () => Response.json({ blocked: false });
  const stopNew = start(false, "/dashboard/skills");
  await settle();
  finishBody({ blocked: true });
  await settle();
  assert.equal(refreshes.length, 5, "an old response body cannot act on the new route/state");
  reply = async () => Response.json({ blocked: true });
  await tick(); await tick();
  assert.deepEqual(refreshes.slice(5), ["/dashboard"], "new blocking redirects agent surfaces once");
  stopNew();

  const stopBlockedRoute = start(true, "/dashboard/machines/runtime_trial/chat");
  await settle(); await tick();
  assert.deepEqual(refreshes.slice(6), ["/dashboard"], "an initially blocked native frame still redirects");
  stopBlockedRoute();

  reply = async () => Response.json({ blocked: false });
  const stopAcknowledged = start(false);
  await settle(); await tick(); await tick();
  assert.equal(refreshes.length, 7, "acknowledged recovery needs no further refresh");
  stopAcknowledged();
});

test("a failed refresh can retry without overlapping polling or an unbounded loop", async (t) => {
  t.mock.timers.enable({ apis: ["setInterval"] });
  t.mock.method(globalThis, "fetch", async () => Response.json({ blocked: false }));
  let attempts = 0;
  const stop = startTrialAccessMonitor({
    blocked: true, pathname: "/dashboard",
    refresh: () => { attempts++; throw new Error("refresh failed"); },
    replace: () => assert.fail("home should not redirect"),
  });
  await new Promise<void>((resolve) => setImmediate(resolve));
  for (let index = 0; index < 5; index++) {
    t.mock.timers.tick(30_000);
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  assert.equal(attempts, 3);
  stop();
});
