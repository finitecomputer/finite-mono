import assert from "node:assert/strict";
import test from "node:test";

import { CHAT_SIGN_IN_DRAFT_UNSAVED_MESSAGE } from "@/lib/chat-product-copy";
import {
  attemptHostedChatSignIn,
  clearHostedChatDraft,
  currentHostedChatReturnPath,
  hostedChatSignInUrl,
  isHostedChatSessionAuthFailure,
  loadHostedChatDraft,
  redirectToHostedChatSignIn,
  resetHostedChatSignInRedirect,
  restoreHostedChatComposerDraft,
  saveHostedChatDraft,
  shouldAutoRedirectForSessionAuthFailure,
} from "@/lib/hosted-chat-session";

// Mirrors the shape of HostedChatHttpError from the hosted chat provider:
// an Error carrying the HTTP status and the hosted-device retry envelope.
class FakeChatHttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly retryable?: boolean
  ) {
    super(message);
  }
}

const SIGN_IN_REDIRECT_STORAGE_KEY = "fc-hosted-chat-sign-in-redirected-at";

type StubWindow = {
  location: {
    pathname: string;
    search: string;
    assigned: string[];
    assign(url: string): void;
  };
};

function installWindow(
  pathname = "/dashboard/machines/runtime_70aecb4ba75f00f2fc6d/chat",
  search = ""
): StubWindow {
  const stub: StubWindow = {
    location: {
      pathname,
      search,
      assigned: [],
      assign(url: string) {
        stub.location.assigned.push(url);
      },
    },
  };
  (globalThis as { window?: unknown }).window = stub;
  return stub;
}

function removeWindow() {
  delete (globalThis as { window?: unknown }).window;
}

function installSessionStorage(initial: Record<string, string> = {}) {
  const store = new Map(Object.entries(initial));
  (globalThis as { sessionStorage?: unknown }).sessionStorage = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
    removeItem: (key: string) => void store.delete(key),
  };
  return store;
}

function removeSessionStorage() {
  delete (globalThis as { sessionStorage?: unknown }).sessionStorage;
}

function cleanup() {
  removeWindow();
  removeSessionStorage();
  resetHostedChatSignInRedirect();
}

test("only a non-retryable 401 from a chat route is a session auth failure", () => {
  try {
    assert.equal(
      isHostedChatSessionAuthFailure(
        new FakeChatHttpError("Sign in again to use chat.", 401)
      ),
      true
    );
    assert.equal(
      isHostedChatSessionAuthFailure(
        new FakeChatHttpError("Sign in again to finish chat setup.", 401)
      ),
      true
    );
    // A core envelope's own retry decision wins even on 401.
    assert.equal(
      isHostedChatSessionAuthFailure(
        new FakeChatHttpError("Chat is unavailable right now.", 401, true)
      ),
      false
    );
    assert.equal(
      isHostedChatSessionAuthFailure(new FakeChatHttpError("refused", 403)),
      false
    );
    assert.equal(
      isHostedChatSessionAuthFailure(
        new FakeChatHttpError("Finish chat setup to continue.", 409)
      ),
      false
    );
    assert.equal(
      isHostedChatSessionAuthFailure(
        new FakeChatHttpError("Chat is unavailable right now.", 503)
      ),
      false
    );
    // Transport failures carry no status at all.
    assert.equal(isHostedChatSessionAuthFailure(new TypeError("fetch failed")), false);
    assert.equal(isHostedChatSessionAuthFailure(new Error("boom")), false);
    assert.equal(isHostedChatSessionAuthFailure("Sign in again to use chat."), false);
    assert.equal(isHostedChatSessionAuthFailure(null), false);
  } finally {
    cleanup();
  }
});

test("the sign-in URL carries a same-origin-safe return path through /login", () => {
  try {
    const chatPath = "/dashboard/machines/runtime_70aecb4ba75f00f2fc6d/chat";
    assert.equal(
      hostedChatSignInUrl(chatPath),
      `/login?returnTo=${encodeURIComponent(chatPath)}`
    );
    assert.equal(
      hostedChatSignInUrl("/dashboard/machines/m/chat?topic=home"),
      `/login?returnTo=${encodeURIComponent("/dashboard/machines/m/chat?topic=home")}`
    );
    // Open-redirect shapes fall back to the dashboard instead of the chat.
    assert.equal(hostedChatSignInUrl("https://evil.example/chat"), "/login?returnTo=%2Fdashboard");
    assert.equal(hostedChatSignInUrl("//evil.example"), "/login?returnTo=%2Fdashboard");
    assert.equal(hostedChatSignInUrl(""), "/login?returnTo=%2Fdashboard");
    assert.equal(hostedChatSignInUrl(null), "/login?returnTo=%2Fdashboard");
  } finally {
    cleanup();
  }
});

test("the return path is the current chat page including its query", () => {
  try {
    installWindow("/dashboard/machines/runtime_a/chat", "?x=1");
    assert.equal(
      currentHostedChatReturnPath(),
      "/dashboard/machines/runtime_a/chat?x=1"
    );
  } finally {
    cleanup();
  }
});

test("an expired session redirects by full-page navigation at most once per page load", () => {
  try {
    installSessionStorage();
    const window = installWindow();
    assert.equal(redirectToHostedChatSignIn(currentHostedChatReturnPath()), true);
    assert.deepEqual(window.location.assigned, [
      hostedChatSignInUrl("/dashboard/machines/runtime_70aecb4ba75f00f2fc6d/chat"),
    ]);
    // A second 401 observed on the same page load (claim, send, SSE follow-up)
    // must not navigate again.
    assert.equal(redirectToHostedChatSignIn(currentHostedChatReturnPath()), false);
    assert.equal(window.location.assigned.length, 1);
  } finally {
    cleanup();
  }
});

test("the banner's explicit sign-in button always navigates", () => {
  try {
    installSessionStorage();
    const window = installWindow();
    redirectToHostedChatSignIn(currentHostedChatReturnPath());
    assert.equal(
      redirectToHostedChatSignIn(currentHostedChatReturnPath(), { force: true }),
      true
    );
    assert.equal(window.location.assigned.length, 2);
  } finally {
    cleanup();
  }
});

test("a return trip that immediately 401s again does not bounce forever", () => {
  try {
    const store = installSessionStorage();
    const firstLoad = installWindow();
    assert.equal(redirectToHostedChatSignIn(currentHostedChatReturnPath()), true);
    assert.equal(firstLoad.location.assigned.length, 1);

    // Fresh page load after a zero-interaction round trip: the per-page-load
    // flag is gone, but the marker left by the redirect that just departed is
    // still inside the bounce window, so the automatic redirect stands down
    // and the "Sign in again" banner is the surface instead.
    resetHostedChatSignInRedirect();
    store.set(SIGN_IN_REDIRECT_STORAGE_KEY, String(Date.now()));
    const returnTrip = installWindow();
    assert.equal(redirectToHostedChatSignIn(currentHostedChatReturnPath()), false);
    assert.equal(returnTrip.location.assigned.length, 0);
    // The explicit button still works even inside the window.
    assert.equal(
      redirectToHostedChatSignIn(currentHostedChatReturnPath(), { force: true }),
      true
    );
    assert.equal(returnTrip.location.assigned.length, 1);
  } finally {
    cleanup();
  }
});

test("a stale bounce marker from an earlier visit does not block a fresh redirect", () => {
  try {
    installSessionStorage({
      [SIGN_IN_REDIRECT_STORAGE_KEY]: String(Date.now() - 60_000),
    });
    const window = installWindow();
    assert.equal(redirectToHostedChatSignIn(currentHostedChatReturnPath()), true);
    assert.equal(window.location.assigned.length, 1);
  } finally {
    cleanup();
  }
});

test("an empty composer may auto-redirect; unsent input keeps the composer", () => {
  assert.equal(
    shouldAutoRedirectForSessionAuthFailure({ draftText: "", attachmentCount: 0 }),
    true
  );
  // Whitespace-only text is not unsent input.
  assert.equal(
    shouldAutoRedirectForSessionAuthFailure({ draftText: "  \n\t", attachmentCount: 0 }),
    true
  );
  assert.equal(
    shouldAutoRedirectForSessionAuthFailure({
      draftText: "Keep the runtime boundary thin.",
      attachmentCount: 0,
    }),
    false
  );
  // Staged attachments alone also pin the composer: files cannot survive a
  // full-page navigation, so only an explicit "Sign in again" may drop them.
  assert.equal(
    shouldAutoRedirectForSessionAuthFailure({ draftText: "", attachmentCount: 2 }),
    false
  );
  assert.equal(
    shouldAutoRedirectForSessionAuthFailure({
      draftText: "half-written",
      attachmentCount: 1,
    }),
    false
  );
});

function draftStorage() {
  const store = new Map<string, string>();
  return {
    store,
    storage: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
      removeItem: (key: string) => void store.delete(key),
    },
  } as const;
}

test("a parked draft round-trips through the sign-in trip once", () => {
  const { storage } = draftStorage();
  const machineId = "runtime_70aecb4ba75f00f2fc6d";
  const nowMs = Date.now();

  assert.equal(saveHostedChatDraft(machineId, "Keep the runtime boundary thin.", { nowMs, storage }), true);
  assert.equal(loadHostedChatDraft(machineId, { nowMs: nowMs + 5_000, storage }), "Keep the runtime boundary thin.");
  // Consuming is one-shot: the caller clears after restoring.
  clearHostedChatDraft(machineId, { storage });
  assert.equal(loadHostedChatDraft(machineId, { nowMs, storage }), null);

  // A blank draft is not parked — it clears whatever was there.
  saveHostedChatDraft(machineId, "old draft", { nowMs, storage });
  assert.equal(saveHostedChatDraft(machineId, "   ", { nowMs, storage }), true);
  assert.equal(loadHostedChatDraft(machineId, { nowMs, storage }), null);

  // Drafts are per machine.
  saveHostedChatDraft(machineId, "for boss", { nowMs, storage });
  assert.equal(loadHostedChatDraft("runtime_other", { nowMs, storage }), null);

  // Machine ids are encoded into distinct, prefix-scoped keys.
  saveHostedChatDraft("runtime other/odd", "scoped", { nowMs, storage });
  assert.equal(loadHostedChatDraft("runtime other/odd", { nowMs, storage }), "scoped");
});

test("an expired or corrupted parked draft restores nothing and clears the key", () => {
  const { store, storage } = draftStorage();
  const machineId = "runtime_70aecb4ba75f00f2fc6d";
  const nowMs = Date.now();

  saveHostedChatDraft(machineId, "stale", { nowMs: nowMs - 16 * 60_000, storage });
  assert.equal(loadHostedChatDraft(machineId, { nowMs, storage }), null);
  assert.equal(store.has(`finite.chat-draft.${machineId}`), false, "expired draft key removed");

  store.set(`finite.chat-draft.${machineId}`, "{not json");
  assert.equal(loadHostedChatDraft(machineId, { nowMs, storage }), null);
  assert.equal(store.has(`finite.chat-draft.${machineId}`), false, "corrupt draft key removed");

  store.set(
    `finite.chat-draft.${machineId}`,
    JSON.stringify({ text: "   ", savedAtMs: nowMs })
  );
  assert.equal(loadHostedChatDraft(machineId, { nowMs, storage }), null, "blank stored text restores nothing");
});

test("a draft that cannot be parked blocks the sign-in trip", () => {
  try {
    installSessionStorage();
    const window = installWindow();
    const machineId = "runtime_70aecb4ba75f00f2fc6d";

    // Storage whose writes throw (quota exhausted): keep the composer.
    const { store } = draftStorage();
    const quotaStorage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: () => {
        throw new Error("quota exceeded");
      },
      removeItem: (key: string) => void store.delete(key),
    };
    assert.equal(
      attemptHostedChatSignIn(machineId, "Keep the runtime boundary thin.", { storage: quotaStorage }),
      CHAT_SIGN_IN_DRAFT_UNSAVED_MESSAGE
    );
    assert.equal(window.location.assigned.length, 0, "no navigation without a parked draft");

    // Over-limit content is unparkable too: save and restore share one shape
    // rule, so it fails closed instead of stranding a draft restore deletes.
    const { storage } = draftStorage();
    assert.equal(saveHostedChatDraft(machineId, "x".repeat(65_537), { storage }), false);
    assert.equal(
      attemptHostedChatSignIn(machineId, "y".repeat(65_537), { storage }),
      CHAT_SIGN_IN_DRAFT_UNSAVED_MESSAGE
    );
    assert.equal(window.location.assigned.length, 0);
    // At the limit the round trip works end to end.
    assert.equal(saveHostedChatDraft(machineId, "x".repeat(65_536), { storage }), true);
    assert.equal(loadHostedChatDraft(machineId, { storage }), "x".repeat(65_536));

    // Blank text has nothing to lose, so even broken storage proceeds.
    assert.equal(attemptHostedChatSignIn(machineId, "   ", { storage: quotaStorage }), null);
    assert.equal(window.location.assigned.length, 1);
    // A parked draft proceeds too.
    assert.equal(
      attemptHostedChatSignIn(machineId, "Keep the runtime boundary thin.", { storage }),
      null
    );
    assert.equal(window.location.assigned.length, 2);
  } finally {
    cleanup();
  }
});

test("a parked draft outranks the ?prompt= on the sign-in return trip", () => {
  const { storage } = draftStorage();
  const machineId = "runtime_70aecb4ba75f00f2fc6d";
  const nowMs = Date.now();

  // Both present: the parked edits resume, the stale prompt is ignored, and
  // the key is consumed.
  saveHostedChatDraft(machineId, "edited after the prompt", { nowMs, storage });
  assert.equal(
    restoreHostedChatComposerDraft(machineId, "original prompt", { nowMs, storage }),
    "edited after the prompt"
  );
  assert.equal(loadHostedChatDraft(machineId, { nowMs, storage }), null, "parked draft consumed");

  // Nothing parked: the ?prompt= prefills as usual.
  assert.equal(
    restoreHostedChatComposerDraft(machineId, "fresh prompt", { nowMs, storage }),
    "fresh prompt"
  );
});
