import assert from "node:assert/strict";
import test from "node:test";

import {
  exchangeOpenRouterCode,
  generateOpenRouterCodeVerifier,
  openRouterAuthorizationUrl,
  openRouterCallbackUrl,
  openRouterCodeChallenge,
  openRouterOAuthConfigured,
  sealOpenRouterState,
  unsealOpenRouterState,
} from "@/lib/openrouter-oauth";

const env = {
  WORKOS_COOKIE_PASSWORD: "12345678901234567890123456789012",
};

test("OpenRouter sign-in derives the S256 PKCE pair", async () => {
  // Known answer from RFC 7636 appendix B.
  assert.equal(
    await openRouterCodeChallenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
    "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
  );
  const verifier = await generateOpenRouterCodeVerifier();
  assert.match(verifier, /^[A-Za-z0-9\-._~]{43,128}$/u);
});

test("OpenRouter authorization URL carries the callback and code challenge", () => {
  const authorization = openRouterAuthorizationUrl(
    "https://finite.computer/openrouter/callback",
    "challenge-value"
  );
  assert.equal(`${authorization.origin}${authorization.pathname}`, "https://openrouter.ai/auth");
  assert.equal(
    authorization.searchParams.get("callback_url"),
    "https://finite.computer/openrouter/callback"
  );
  assert.equal(authorization.searchParams.get("code_challenge"), "challenge-value");
  assert.equal(authorization.searchParams.get("code_challenge_method"), "S256");
});

test("OpenRouter callback resolves from the configured public origin", () => {
  const productionEnv = {
    ...env,
    FC_DASHBOARD_BASE_URL: "https://finite.computer",
    NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://finite.computer/callback",
  };
  assert.equal(
    openRouterCallbackUrl("http://127.0.0.1:3000/internal", productionEnv),
    "https://finite.computer/openrouter/callback"
  );
  assert.equal(openRouterOAuthConfigured(env), true);
  assert.equal(
    openRouterOAuthConfigured({ ...env, WORKOS_COOKIE_PASSWORD: "short" }),
    false
  );
});

test("OpenRouter state seals the verifier bound to a machine and user", async () => {
  const state = {
    machineId: "machine-a",
    workosUserId: "user-a",
    codeVerifier: await generateOpenRouterCodeVerifier(),
    issuedAtMs: Date.now(),
  };
  const sealed = await sealOpenRouterState(state, env);
  assert.deepEqual(await unsealOpenRouterState(sealed, env), state);
  assert.equal(
    await unsealOpenRouterState(sealed, {
      ...env,
      WORKOS_COOKIE_PASSWORD: "abcdefghijklmnopqrstuvwxyz123456",
    }),
    null
  );
  assert.equal(await unsealOpenRouterState("not-sealed", env), null);
  assert.equal(
    await unsealOpenRouterState(
      await sealOpenRouterState(
        { ...state, issuedAtMs: Date.now() - 16 * 60 * 1000 },
        env
      ),
      env
    ),
    null
  );
  assert.equal(
    await unsealOpenRouterState(
      await sealOpenRouterState({ ...state, codeVerifier: "too-short" }, env),
      env
    ),
    null
  );
});

test("OpenRouter key exchange returns the key only on a well-formed response", async () => {
  const ok = (async () =>
    new Response(JSON.stringify({ key: "sk-or-v1-exchanged-key-value" }), {
      status: 200,
    })) as typeof fetch;
  assert.equal(await exchangeOpenRouterCode("code-a", "verifier-a", ok), "sk-or-v1-exchanged-key-value");
  const rejected = (async () =>
    new Response(JSON.stringify({ error: "invalid code" }), { status: 403 })) as typeof fetch;
  assert.equal(await exchangeOpenRouterCode("code-a", "verifier-a", rejected), null);
  const truncated = (async () =>
    new Response(JSON.stringify({ key: "short" }), { status: 200 })) as typeof fetch;
  assert.equal(await exchangeOpenRouterCode("code-a", "verifier-a", truncated), null);
});
