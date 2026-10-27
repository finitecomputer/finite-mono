/** Opt-in proof: the dashboard deletion route against the real hosted device
 * and Brain server binaries. Build them, then run with
 * FC_BRAIN_DELETION_PROOF_BIN_DIR=<cargo target/debug>; set
 * FC_BRAIN_DELETION_PROOF_REPORT=<path> to keep a JSON evidence report.
 * Synthetic Brains and accounts only. */
import assert from "node:assert/strict";
import { type ChildProcess, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, writeFile } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { renameBrainFixtureEvent } from "../../browser/brain-deletion-fixture";

const BIN_DIR = process.env.FC_BRAIN_DELETION_PROOF_BIN_DIR?.trim();
const TOKEN = "synthetic-hosted-device-token";
const DASHBOARD = "http://dashboard.test";

async function freePort() {
  const server = http.createServer().listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  await new Promise<void>((resolve) => server.close(() => resolve()));
  return address.port;
}

async function waitFor(what: string, child: ChildProcess, ready: () => Promise<boolean>) {
  const deadline = Date.now() + 30_000;
  while (!(await ready().catch(() => false))) {
    assert.equal(child.exitCode, null, `${what} exited`);
    assert(Date.now() < deadline, `${what} did not become ready`);
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

function asUser(workosUserId: string) {
  process.env.FC_DASHBOARD_DEV_WORKOS_USER_ID = workosUserId;
  process.env.FC_DASHBOARD_DEV_EMAIL = `${workosUserId}@example.test`;
}

test("dashboard Brain deletion signs through the hosted device and the Brain server enforces it", {
  skip: BIN_DIR ? false : "set FC_BRAIN_DELETION_PROOF_BIN_DIR to the built binaries",
  timeout: 120_000,
}, async () => {
  const scratch = await mkdtemp(path.join(os.tmpdir(), "brain-deletion-proof-"));
  const [brainPort, devicePort] = [await freePort(), await freePort()];
  const brainOrigin = `http://127.0.0.1:${brainPort}`;
  const children: ChildProcess[] = [];
  const evidence: Record<string, unknown> = {};
  try {
    const brain = spawn(path.join(BIN_DIR!, "finite-brain"), [], {
      env: {
        NODE_ENV: "test",
        FINITE_BRAIN_ADDR: `127.0.0.1:${brainPort}`,
        FINITE_BRAIN_PUBLIC_BASE_URL: brainOrigin,
        FINITE_BRAIN_DB: path.join(scratch, "brain.sqlite3"),
      },
      stdio: "ignore",
    });
    const device = spawn(path.join(BIN_DIR!, "finitechat-hosted-device"), [
      "--bind", `127.0.0.1:${devicePort}`,
      "--data-root", path.join(scratch, "hosted"),
      "--server-url", "http://127.0.0.1:9",
      "--public-url", "http://chat.test",
    ], { env: { NODE_ENV: "test", FINITECHAT_HOSTED_API_TOKEN: TOKEN }, stdio: "ignore" });
    children.push(brain, device);
    await waitFor("Brain server", brain, async () => (await fetch(`${brainOrigin}/health`)).ok);
    const deviceUrl = `http://127.0.0.1:${devicePort}`;
    const provision = (user: string) => fetch(`${deviceUrl}/v1/app/state`, {
      headers: { authorization: `Bearer ${TOKEN}`, "x-finite-workos-user-id": user },
    });
    await waitFor("hosted device", device, async () => (await provision("user_admin")).ok);
    assert.equal((await provision("user_outsider")).status, 200);

    Object.assign(process.env, {
      FC_HOSTED_WEB_DEVICE_URL: deviceUrl,
      FINITECHAT_HOSTED_API_TOKEN: TOKEN,
      FC_BRAIN_UPSTREAM_URL: brainOrigin,
      FC_BRAIN_PUBLIC_ORIGIN: brainOrigin,
      FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH: "1",
      FC_DASHBOARD_DEV_WORKOS_ACCESS_TOKEN: "synthetic-access-token",
    });
    delete process.env.FC_WORKOS_AUTH_ENABLED;
    asUser("user_admin");
    const route = await import("../app/api/brain/deletion/route");
    const { getAccountAuthContext } = await import("./dashboard-auth");
    const { hostedDeviceConfig } = await import("./hosted-web-device");
    const { hostedBrainDeletionTransport } = await import("./brain-deletion");
    const adminTransport = hostedBrainDeletionTransport(hostedDeviceConfig()!, await getAccountAuthContext(), brainOrigin);
    const { npub: adminNpub } = await adminTransport.identify();
    for (const [brainId, name] of [["v1", "V1"], ["content-brain", "Content"], ["marketing-comms-brain", "Marketing Comms"]]) {
      await adminTransport.signedRequest("POST", "/v1/brains", JSON.stringify({ brainId, kind: "organization", name }));
    }

    const preview = (user: string) => {
      asUser(user);
      return route.GET(new Request(`${DASHBOARD}/api/brain/deletion?brainId=v1`, { headers: { host: "dashboard.test" } }));
    };
    const confirm = (body: Record<string, unknown>) => {
      asUser("user_admin");
      return route.POST(new Request(`${DASHBOARD}/api/brain/deletion`, {
        method: "POST",
        headers: { host: "dashboard.test", origin: DASHBOARD, "content-type": "application/json" },
        body: JSON.stringify(body),
      }));
    };

    // Only a current admin may preview; the preview names the server.
    assert.equal((await preview("user_outsider")).status, 403);
    const first = await preview("user_admin");
    assert.equal(first.status, 200);
    const firstScope = await first.json();
    assert.equal(firstScope.server, brainOrigin);
    assert.equal(firstScope.name, "V1");
    evidence.adminPreview = { server: firstScope.server, brainId: firstScope.brainId, deletable: firstScope.deletable };

    // A rename after the preview invalidates it; a mistyped confirmation never signs.
    const renameEvent = await adminTransport.signBrainEvent(renameBrainFixtureEvent({
      brainId: "v1", adminNpub, note: "V1 (renamed)",
      changeId: "rename-after-preview", createdAt: Math.floor(Date.now() / 1000),
    }));
    await adminTransport.signedRequest("POST", "/v1/brains/v1/rename", JSON.stringify({ accessChangeEvent: renameEvent }));
    const stale = await confirm({ brainId: "v1", stateDigest: firstScope.stateDigest, confirmation: "v1" });
    assert.equal(stale.status, 409);
    assert.match((await stale.json()).error, /changed after its deletion preview/u);
    assert.equal((await confirm({ brainId: "v1", stateDigest: firstScope.stateDigest, confirmation: "V1" })).status, 400);

    // Confirm the fresh preview while the first DELETE response is lost in
    // transit: the route resends the identical signed deletion once.
    const fresh = await (await preview("user_admin")).json();
    assert.notEqual(fresh.stateDigest, firstScope.stateDigest);
    const realFetch = globalThis.fetch;
    let dropped = 0;
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const response = await realFetch(input, init);
      if (String(input) === `${brainOrigin}/v1/brains/v1` && init?.method === "DELETE" && dropped === 0) {
        dropped += 1;
        await response.text();
        throw new TypeError("fetch failed: response lost in transit");
      }
      return response;
    }) as typeof fetch;
    let deleted: Response;
    try {
      deleted = await confirm({ brainId: "v1", stateDigest: fresh.stateDigest, confirmation: "v1" });
    } finally {
      globalThis.fetch = realFetch;
    }
    assert.equal(dropped, 1);
    assert.equal(deleted.status, 200);
    const result = await deleted.json();
    assert.equal(result.duplicate, true);
    assert.equal(result.receipt.brainId, "v1");
    assert.equal(result.receipt.actorNpub, adminNpub);
    assert.equal(result.receipt.stateDigest, fresh.stateDigest);
    evidence.deletedThroughRoute = { receipt: result.receipt, exactRetryAfterLostResponse: result.duplicate };

    // The former admin learns it is gone; an unrelated account cannot tell.
    const gone = await preview("user_admin");
    assert.equal(gone.status, 410);
    assert.match((await gone.json()).error, /permanently deleted/u);
    assert.equal((await preview("user_outsider")).status, 404);
    asUser("user_admin");
    const listed = await adminTransport.signedRequest("GET", "/v1/brains") as { brains: Array<{ brainId: string }> };
    assert.deepEqual(listed.brains.map((brain) => brain.brainId).sort(), ["content-brain", "marketing-comms-brain"]);
    evidence.afterDeletion = { formerAdmin: 410, outsider: 404, remaining: listed.brains.map((brain) => brain.brainId).sort() };
  } finally {
    for (const child of children) child.kill("SIGTERM");
    const report = process.env.FC_BRAIN_DELETION_PROOF_REPORT;
    if (report) await writeFile(report, JSON.stringify({ format: "finite.dashboard.brain-deletion-hosted-proof.v1", evidence }, null, 2));
  }
});
