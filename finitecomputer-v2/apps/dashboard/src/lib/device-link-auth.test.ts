import assert from "node:assert/strict";
import test from "node:test";

import {
  SignJWT,
  createLocalJWKSet,
  exportJWK,
  generateKeyPair,
} from "jose";

import { DeviceLinkError } from "@/lib/device-link";
import {
  getDeviceLinkAccountAuthContext,
  parseBearerToken,
  verifiedBearerClaims,
  verifyDeviceLinkBearerToken,
} from "@/lib/device-link-auth";

const CLIENT_ID = "client_test_finite";
const USER_ID = "user_01HBEQKA6K4QJAS93VPE39W1JT";
const SESSION_ID = "session_01HQSXZGF8FHF7A9ZZFCW4387R";

test("native device-link bearer tokens require a strict Authorization value", () => {
  assert.equal(parseBearerToken("Bearer aaa.bbb.ccc"), "aaa.bbb.ccc");
  for (const value of [
    "bearer aaa.bbb.ccc",
    "Bearer  aaa.bbb.ccc",
    "Bearer aaa.bbb.ccc ",
    "Bearer aaa.bbb",
    "Bearer aaa.bbb.ccc\n",
    `Bearer ${"a".repeat(17_000)}.bbb.ccc`,
  ]) {
    assert.throws(
      () => parseBearerToken(value),
      (error: unknown) => error instanceof DeviceLinkError && error.status === 401
    );
  }
});

test("native device-link bearer claims bind the exact WorkOS application", () => {
  assert.deepEqual(
    verifiedBearerClaims(
      {
        sub: USER_ID,
        sid: SESSION_ID,
        jti: "jwt_123",
        client_id: CLIENT_ID,
        org_id: "org_123",
      },
      CLIENT_ID
    ),
    {
      subject: USER_ID,
      organizationId: "org_123",
    }
  );
  assert.throws(() =>
    verifiedBearerClaims(
      {
        sub: USER_ID,
        sid: SESSION_ID,
        jti: "jwt_123",
        client_id: "client_other",
      },
      CLIENT_ID
    )
  );
});

test("native device-link bearer verification checks signature and expiry", async () => {
  const { publicKey, privateKey } = await generateKeyPair("RS256");
  const publicJwk = await exportJWK(publicKey);
  publicJwk.kid = "test-key";
  publicJwk.alg = "RS256";
  const keys = createLocalJWKSet({ keys: [publicJwk] });
  const now = Math.floor(Date.now() / 1000);
  const token = await new SignJWT({
    client_id: CLIENT_ID,
    sid: SESSION_ID,
    jti: "jwt_123",
  })
    .setProtectedHeader({ alg: "RS256", kid: "test-key" })
    .setIssuer("https://api.workos.com")
    .setSubject(USER_ID)
    .setIssuedAt(now)
    .setExpirationTime(now + 60)
    .sign(privateKey);

  assert.deepEqual(await verifyDeviceLinkBearerToken(token, CLIENT_ID, keys), {
    subject: USER_ID,
    organizationId: null,
  });
  await assert.rejects(() =>
    verifyDeviceLinkBearerToken(token, "client_wrong", keys)
  );

  const expired = await new SignJWT({
    client_id: CLIENT_ID,
    sid: SESSION_ID,
    jti: "jwt_expired",
  })
    .setProtectedHeader({ alg: "RS256", kid: "test-key" })
    .setIssuer("https://api.workos.com")
    .setSubject(USER_ID)
    .setIssuedAt(now - 120)
    .setExpirationTime(now - 60)
    .sign(privateKey);
  await assert.rejects(() =>
    verifyDeviceLinkBearerToken(expired, CLIENT_ID, keys)
  );

  const otherKeys = await generateKeyPair("RS256");
  const badSignature = await new SignJWT({
    client_id: CLIENT_ID,
    sid: SESSION_ID,
    jti: "jwt_bad_signature",
  })
    .setProtectedHeader({ alg: "RS256", kid: "test-key" })
    .setIssuer("https://api.workos.com")
    .setSubject(USER_ID)
    .setIssuedAt(now)
    .setExpirationTime(now + 60)
    .sign(otherKeys.privateKey);
  await assert.rejects(() =>
    verifyDeviceLinkBearerToken(badSignature, CLIENT_ID, keys)
  );
});

test("native bearer accepts a signed WorkOS custom issuer", async () => {
  const { publicKey, privateKey } = await generateKeyPair("RS256");
  const publicJwk = await exportJWK(publicKey);
  publicJwk.kid = "test-key";
  publicJwk.alg = "RS256";
  const keys = createLocalJWKSet({ keys: [publicJwk] });
  const now = Math.floor(Date.now() / 1000);
  const token = await new SignJWT({
    client_id: CLIENT_ID,
    sid: SESSION_ID,
    jti: "jwt_custom_issuer",
  })
    .setProtectedHeader({ alg: "RS256", kid: "test-key" })
    .setIssuer("https://auth.example.com")
    .setSubject(USER_ID)
    .setIssuedAt(now)
    .setExpirationTime(now + 60)
    .sign(privateKey);

  assert.deepEqual(await verifyDeviceLinkBearerToken(token, CLIENT_ID, keys), {
    subject: USER_ID,
    organizationId: null,
  });
});

test("native bearer context resolves the verified WorkOS user", async () => {
  const request = new Request(
    "https://finite.computer/api/device-links/approve",
    { headers: { authorization: "Bearer aaa.bbb.ccc" } }
  );
  const account = await getDeviceLinkAccountAuthContext(
    request,
    {
      FC_WORKOS_IOS_CLIENT_ID: CLIENT_ID,
      WORKOS_API_KEY: "sk_test_server_only",
    },
    {
      async verify(token, clientId) {
        assert.equal(token, "aaa.bbb.ccc");
        assert.equal(clientId, CLIENT_ID);
        return { subject: USER_ID, organizationId: "org_123" };
      },
      async getUser(userId, apiKey) {
        assert.equal(userId, USER_ID);
        assert.equal(apiKey, "sk_test_server_only");
        return {
          id: USER_ID,
          email: "paul@finite.computer",
          emailVerified: true,
        };
      },
    }
  );

  assert.deepEqual(account, {
    email: "paul@finite.computer",
    workosUserId: USER_ID,
    emailVerified: true,
    organizationId: "org_123",
    source: "workos",
  });
});
