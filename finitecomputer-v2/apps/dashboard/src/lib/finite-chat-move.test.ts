import assert from "node:assert/strict";
import { test } from "node:test";

import { FINITE_CHAT_URL, finiteChatUrl, movedRedirect, phaseTwoRedirect, redirectAllToFiniteChat } from "./finite-chat-move";

test("a person goes to finite.chat only once Core says every agent of theirs has moved", () => {
  const person = { isAdmin: false };
  assert.equal(movedRedirect({ moved_to_finite_chat: true }, person, {}), FINITE_CHAT_URL);
  assert.equal(movedRedirect({ moved_to_finite_chat: false }, person, {}), null);
  // An older Core leaves the field out: nobody moves.
  assert.equal(movedRedirect({}, person, {}), null);
  assert.equal(movedRedirect(null, person, {}), null);
  // An operator whose own agents moved still runs the fleet here.
  assert.equal(movedRedirect({ moved_to_finite_chat: true }, { isAdmin: true }, {}), null);
});

test("the phase 2 switch is off unless it is set", () => {
  assert.equal(redirectAllToFiniteChat({}), false);
  assert.equal(redirectAllToFiniteChat({ FC_REDIRECT_ALL_TO_FINITE_CHAT: "0" }), false);
  assert.equal(redirectAllToFiniteChat({ FC_REDIRECT_ALL_TO_FINITE_CHAT: " On " }), true);
  assert.equal(phaseTwoRedirect("/dashboard", "GET", {}), null);
});

test("phase 2 sends every page there and keeps machine routes answering", () => {
  const env = { FC_REDIRECT_ALL_TO_FINITE_CHAT: "1" };
  // `/code` is the launch-code form for a new agent here.
  for (const page of ["/", "/signup", "/code", "/dashboard", "/dashboard/machines/runtime_1", "/dashboard/administration", "/healthcheck"]) {
    assert.equal(phaseTwoRedirect(page, "GET", env), FINITE_CHAT_URL, page);
  }
  assert.equal(phaseTwoRedirect("/dashboard", "HEAD", env), FINITE_CHAT_URL);
  for (const kept of [
    "/api/stripe/webhook",
    "/api/core/v1/me",
    "/health",
    "/healthz",
    "/callback",
    "/site-auth",
    "/google-workspace/callback",
    "/login",
    "/logout",
    "/dashboard/admin",
    "/dashboard/admin/runtime-upgrade",
  ]) {
    assert.equal(phaseTwoRedirect(kept, "GET", env), null, kept);
  }
  // A form or server action is never turned into a GET elsewhere.
  assert.equal(phaseTwoRedirect("/dashboard", "POST", env), null);
});

test("the destination is finite.chat unless configured, and a bad one is refused", () => {
  assert.equal(finiteChatUrl({}), "https://finite.chat/");
  assert.equal(finiteChatUrl({ FC_FINITE_CHAT_URL: "https://preview.finite.chat" }), "https://preview.finite.chat/");
  assert.throws(() => finiteChatUrl({ FC_FINITE_CHAT_URL: "finite.chat" }), /not an http\(s\) URL/);
  assert.throws(() => finiteChatUrl({ FC_FINITE_CHAT_URL: "javascript:alert(1)" }), /not an http\(s\) URL/);
});
