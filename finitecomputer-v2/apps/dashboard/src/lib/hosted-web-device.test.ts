import assert from "node:assert/strict";
import test from "node:test";

import {
  hostedDeviceAttachments,
  hostedDeviceConfig,
  hostedDeviceHeaders,
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
    /verified WorkOS account/
  );
});
