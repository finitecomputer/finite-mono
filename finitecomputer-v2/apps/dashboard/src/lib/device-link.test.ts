import assert from "node:assert/strict";
import test from "node:test";

import {
  DeviceLinkError,
  deviceLinkBoundaryError,
  parseDeviceLinkRequest,
} from "@/lib/device-link";
import { HostedDeviceRequestError } from "@/lib/hosted-web-device";

test("device-link requests bind one exact session to one distinct Device", () => {
  assert.deepEqual(
    parseDeviceLinkRequest({
      link_session_id: "link-alpha-01",
      target_device_id: "electron-paul-01",
    }),
    {
      link_session_id: "link-alpha-01",
      target_device_id: "electron-paul-01",
    }
  );

  for (const value of [
    null,
    {},
    { link_session_id: " link-alpha", target_device_id: "electron-alpha" },
    { link_session_id: "link-alpha", target_device_id: "hosted-web" },
    { link_session_id: "link-alpha\n", target_device_id: "electron-alpha" },
    { link_session_id: "link-alpha", target_device_id: "x".repeat(257) },
  ]) {
    assert.throws(
      () => parseDeviceLinkRequest(value),
      (error: unknown) => error instanceof DeviceLinkError && error.status === 400
    );
  }
});

test("device-link boundary preserves user errors and hides internal failures", () => {
  const missing = deviceLinkBoundaryError(
    new HostedDeviceRequestError("device link was not found", 404)
  );
  assert.equal(missing.status, 404);
  assert.equal(missing.message, "device link was not found");

  for (const error of [
    new HostedDeviceRequestError("connect ECONNREFUSED 10.0.0.4:38918", 503),
    new Error("fetch failed for secret.internal"),
  ]) {
    const safe = deviceLinkBoundaryError(error);
    assert.equal(safe.status, 502);
    assert.equal(safe.message, "Device linking is unavailable right now.");
  }
});
