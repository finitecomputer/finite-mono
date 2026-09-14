import assert from "node:assert/strict";
import test from "node:test";

import {
  currentHostedChatReturnPath,
  hostedChatSignInUrl,
  isHostedChatSessionAuthFailure,
  redirectToHostedChatSignIn,
  resetHostedChatSignInRedirect,
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
