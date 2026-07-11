import assert from "node:assert/strict";
import test from "node:test";

import {
  hostedDeviceApproveLink,
  hostedDeviceAttachments,
  hostedDeviceConfig,
  hostedDeviceHeaders,
  hostedDeviceProfileImage,
  hostedDeviceLinkStatus,
  hostedDeviceRuntimeCommand,
} from "@/lib/hosted-web-device";

const verifiedAccount = {
  email: "paul@finite.vip",
  workosUserId: "user_paul",
  emailVerified: true,
  source: "workos" as const,
};

test("hostedDeviceConfig requires both internal endpoint and token", () => {
  assert.equal(hostedDeviceConfig({}), null);
  assert.equal(
    hostedDeviceConfig({ FC_HOSTED_WEB_DEVICE_URL: "https://device.internal" }),
    null
  );
  assert.deepEqual(
    hostedDeviceConfig({
      FC_HOSTED_WEB_DEVICE_URL: "https://device.internal/",
      FINITECHAT_HOSTED_API_TOKEN: "secret",
    }),
    { baseUrl: "https://device.internal", apiToken: "secret" }
  );
});

test("hostedDeviceHeaders binds the internal call to the verified WorkOS user", () => {
  const headers = hostedDeviceHeaders(
    { baseUrl: "https://device.internal", apiToken: "secret" },
    verifiedAccount
  );
  assert.equal(headers.get("authorization"), "Bearer secret");
  assert.equal(headers.get("x-finite-workos-user-id"), "user_paul");
});

test("hostedDeviceAttachments preserves the browser multipart boundary", async (context) => {
  const originalFetch = globalThis.fetch;
  context.after(() => {
    globalThis.fetch = originalFetch;
  });
  let capturedHeaders: Headers | null = null;
  globalThis.fetch = async (_input, init) => {
    capturedHeaders = new Headers(init?.headers);
    return Response.json({ rev: 1 });
  };

  const formData = new FormData();
  formData.set("room_id", "room-1");
  formData.set("caption", "Screenshot");
  formData.append("files", new Blob(["bytes"], { type: "text/plain" }), "notes.txt");
  await hostedDeviceAttachments(
    { baseUrl: "https://device.internal", apiToken: "secret" },
    verifiedAccount,
    formData
  );

  assert.equal(capturedHeaders?.get("content-type"), null);
  assert.equal(capturedHeaders?.get("x-finite-workos-user-id"), "user_paul");
});

test("hostedDeviceHeaders rejects unverified or header-only identities", () => {
  assert.throws(
    () =>
      hostedDeviceHeaders(
        { baseUrl: "https://device.internal", apiToken: "secret" },
        {
          email: "paul@finite.vip",
          workosUserId: null,
          emailVerified: false,
          source: "header",
        }
      ),
    /Sign in again/
  );
});

test("profile images use the user Device's public image upload", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  let observedUrl = "";
  let observedHeaders = new Headers();
  global.fetch = (async (input, init) => {
    observedUrl = String(input);
    observedHeaders = new Headers(init?.headers);
    return Response.json({ image_url: "https://chat.example/blobs/profile.png" });
  }) as typeof fetch;

  const imageUrl = await hostedDeviceProfileImage(
    { baseUrl: "https://device.internal", apiToken: "internal-token" },
    verifiedAccount,
    new Blob(["png"], { type: "image/png" })
  );

  assert.equal(observedUrl, "https://device.internal/v1/app/images");
  assert.equal(observedHeaders.get("content-type"), "image/png");
  assert.equal(observedHeaders.get("x-finite-workos-user-id"), "user_paul");
  assert.equal(imageUrl, "https://chat.example/blobs/profile.png");
});

test("runtime commands use the narrow hosted-device endpoint", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  let observedUrl = "";
  let observedBody = "";
  global.fetch = (async (input, init) => {
    observedUrl = String(input);
    observedBody = String(init?.body);
    return new Response(
      JSON.stringify({ request_id: "request-a", status: "succeeded", body: { ok: true } }),
      { status: 200, headers: { "content-type": "application/json" } }
    );
  }) as typeof fetch;
  await hostedDeviceRuntimeCommand(
    { baseUrl: "https://device.internal", apiToken: "internal-token" },
    {
      email: "user@example.com",
      workosUserId: "user-a",
      emailVerified: true,
      source: "workos",
    },
    {
      room_id: "room-a",
      target_account_id: "agent-a",
      command: "agent.connections.status",
      schema: "finite.agent.empty.request.v1",
      body: {},
    }
  );
  assert.equal(observedUrl, "https://device.internal/v1/app/runtime-commands");
  assert.match(observedBody, /agent\.connections\.status/u);
});

test("device linking stays server-side and projects only public progress", async (context) => {
  const originalFetch = global.fetch;
  context.after(() => {
    global.fetch = originalFetch;
  });
  const observed: Array<{ url: string; headers: Headers; body: string }> = [];
  global.fetch = (async (input, init) => {
    observed.push({
      url: String(input),
      headers: new Headers(init?.headers),
      body: String(init?.body),
    });
    return Response.json({
      link_session_id: "link-alpha",
      target_device_id: "electron-alpha",
      status: observed.length === 1 ? "awaiting_claim" : "ready",
      expires_at_unix_seconds: 1_800_000_600,
      room_count: 2,
      active_room_count: observed.length === 1 ? 0 : 2,
      account_secret_hex: "must-never-cross-the-dashboard-boundary",
      encrypted_payload: [1, 2, 3],
    });
  }) as typeof fetch;

  const input = {
    link_session_id: "link-alpha",
    target_device_id: "electron-alpha",
  };
  const approved = await hostedDeviceApproveLink(
    { baseUrl: "https://device.internal", apiToken: "internal-token" },
    verifiedAccount,
    input
  );
  const ready = await hostedDeviceLinkStatus(
    { baseUrl: "https://device.internal", apiToken: "internal-token" },
    verifiedAccount,
    input
  );

  assert.deepEqual(approved, {
    ...input,
    status: "awaiting_claim",
    expires_at_unix_seconds: 1_800_000_600,
    room_count: 2,
    active_room_count: 0,
  });
  assert.equal(ready.status, "ready");
  assert.deepEqual(
    observed.map(({ url }) => url),
    [
      "https://device.internal/v1/device-links/approve",
      "https://device.internal/v1/device-links/status",
    ]
  );
  for (const request of observed) {
    assert.equal(request.headers.get("x-finite-workos-user-id"), "user_paul");
    assert.deepEqual(JSON.parse(request.body), input);
  }
  assert.equal("account_secret_hex" in approved, false);
  assert.equal("encrypted_payload" in approved, false);
});
