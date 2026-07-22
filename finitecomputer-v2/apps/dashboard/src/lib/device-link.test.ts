import assert from "node:assert/strict";
import test from "node:test";

import {
  DeviceLinkError,
  MAX_DEVICE_LINK_REQUEST_BYTES,
  deviceLinkBoundaryError,
  deviceLinkRouteError,
  parseDeviceLinkJsonRequest,
  parseDeviceLinkRequest,
  parseOptionalDeviceStatusTarget,
  projectHostedWebAccountBinding,
} from "@/lib/device-link";
import { HostedDeviceRequestError } from "@/lib/hosted-web-device";

const ACCOUNT_ID = "a".repeat(64);

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
    [],
    {},
    {
      link_session_id: "link-alpha",
      target_device_id: "electron-alpha",
      approval_secret: "do-not-accept",
    },
    { link_session_id: " link-alpha", target_device_id: "electron-alpha" },
    { link_session_id: "link-alpha", target_device_id: "hosted-web" },
    { link_session_id: "link-alpha\n", target_device_id: "electron-alpha" },
    { link_session_id: "link-alpha", target_device_id: `electron\u200b-alpha` },
    { link_session_id: "link-alpha", target_device_id: "é".repeat(129) },
  ]) {
    assert.throws(
      () => parseDeviceLinkRequest(value),
      (error: unknown) => error instanceof DeviceLinkError && error.status === 400
    );
  }
});

test("device-link JSON parsing enforces media type and actual byte limits", async () => {
  const valid = new Request("https://finite.computer/api/device-links/approve", {
    method: "POST",
    headers: { "content-type": "application/json; charset=utf-8" },
    body: JSON.stringify({
      link_session_id: "link-alpha-01",
      target_device_id: "electron-paul-01",
    }),
  });
  assert.deepEqual(await parseDeviceLinkJsonRequest(valid), {
    link_session_id: "link-alpha-01",
    target_device_id: "electron-paul-01",
  });

  const wrongMediaType = new Request("https://finite.computer/api/device-links/approve", {
    method: "POST",
    headers: { "content-type": "text/plain" },
    body: "{}",
  });
  await assert.rejects(
    parseDeviceLinkJsonRequest(wrongMediaType),
    (error: unknown) => error instanceof DeviceLinkError && error.status === 415
  );

  const oversizedDeclared = new Request("https://finite.computer/api/device-links/approve", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "content-length": String(MAX_DEVICE_LINK_REQUEST_BYTES + 1),
    },
    body: "{}",
  });
  await assert.rejects(
    parseDeviceLinkJsonRequest(oversizedDeclared),
    (error: unknown) => error instanceof DeviceLinkError && error.status === 413
  );

  const oversizedActual = new Request("https://finite.computer/api/device-links/approve", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: "x".repeat(MAX_DEVICE_LINK_REQUEST_BYTES + 1),
  });
  await assert.rejects(
    parseDeviceLinkJsonRequest(oversizedActual),
    (error: unknown) => error instanceof DeviceLinkError && error.status === 413
  );
});

test("device-link boundary maps upstream failures to fixed public errors", () => {
  const mappings = [
    [400, 400, "This device-link request is invalid."],
    [404, 404, "This device-link request was not found."],
    [409, 409, "This Device cannot be linked from its current state."],
    [410, 410, "This device-link request expired."],
  ] as const;

  for (const [upstreamStatus, status, message] of mappings) {
    const safe = deviceLinkBoundaryError(
      new HostedDeviceRequestError("upstream detail that must not escape", upstreamStatus)
    );
    assert.equal(safe.status, status);
    assert.equal(safe.message, message);
  }

  for (const error of [
    new HostedDeviceRequestError("connect ECONNREFUSED 10.0.0.4:38918", 503),
    new Error("fetch failed for secret.internal"),
  ]) {
    const safe = deviceLinkBoundaryError(error);
    assert.equal(safe.status, 502);
    assert.equal(safe.message, "Device linking is unavailable right now.");
  }

  const routeSafe = deviceLinkRouteError(new Error("secret internal exception"));
  assert.equal(routeSafe.status, 500);
  assert.equal(routeSafe.message, "Device linking is unavailable right now.");
});

test("Hosted Web account binding projects only a valid public Nostr account id", () => {
  const state = {
    identity: {
      account_id: ACCOUNT_ID,
      device_id: "hosted-web",
      account_secret_hex: "must-not-escape",
    },
    api_token: "must-not-escape",
    rooms: [],
    devices: [
      {
        account_id: ACCOUNT_ID,
        device_id: "electron-active",
        active: true,
        current_device: false,
        revoked: false,
        room_count: 2,
        credential: "must-not-escape",
      },
      {
        account_id: ACCOUNT_ID,
        device_id: "electron-revoked",
        active: true,
        current_device: false,
        revoked: true,
        room_count: 2,
      },
    ],
  };
  assert.deepEqual(
    projectHostedWebAccountBinding(state),
    { account_id: ACCOUNT_ID }
  );
  assert.deepEqual(projectHostedWebAccountBinding(state, "electron-active"), {
    account_id: ACCOUNT_ID,
    local_device: { device_id: "electron-active", status: "available" },
  });
  assert.deepEqual(projectHostedWebAccountBinding(state, "electron-revoked"), {
    account_id: ACCOUNT_ID,
    local_device: { device_id: "electron-revoked", status: "revoked" },
  });
  assert.deepEqual(projectHostedWebAccountBinding(state, "electron-new"), {
    account_id: ACCOUNT_ID,
    local_device: { device_id: "electron-new", status: "unknown" },
  });

  for (const value of [
    null,
    {},
    { identity: null },
    { identity: { account_id: "not-a-nostr-account" } },
    { identity: { account_id: "A".repeat(64) } },
  ]) {
    assert.throws(() => projectHostedWebAccountBinding(value), /invalid identity/u);
  }
});

test("Device status lookup accepts one exact optional target and no other query", () => {
  assert.equal(
    parseOptionalDeviceStatusTarget(
      new Request("https://finite.computer/api/device-links/account-binding")
    ),
    undefined
  );
  assert.equal(
    parseOptionalDeviceStatusTarget(
      new Request(
        "https://finite.computer/api/device-links/account-binding?target_device_id=electron-alpha"
      )
    ),
    "electron-alpha"
  );
  for (const url of [
    "https://finite.computer/api/device-links/account-binding?target_device_id=a&target_device_id=b",
    "https://finite.computer/api/device-links/account-binding?unexpected=1",
    "https://finite.computer/api/device-links/account-binding?target_device_id=%20bad",
  ]) {
    assert.throws(
      () => parseOptionalDeviceStatusTarget(new Request(url)),
      (error: unknown) => error instanceof DeviceLinkError && error.status === 400
    );
  }
});
