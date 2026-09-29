import assert from "node:assert/strict";
import test from "node:test";
import { createElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import {
  InferenceConnections,
  NoticeLine,
  watchState,
  type InferenceNotice,
} from "@/components/inference-connections";
import {
  OpenRouterConnection,
  OpenRouterRemoved,
  submitOpenRouterKey,
} from "@/components/openrouter-connection";
import { parseConnectionsStatus } from "@/lib/hosted-agent-controls";
import { inferenceView } from "@/lib/inference-status";

const ALL_CAPABILITIES = [
  "inference.status.v2",
  "inference.select.v1",
  "inference.disconnect.v1",
  "openrouter.connect.v1",
  "openrouter.usage.v1",
  "codex.login.v1",
  "codex.models.v1",
];
const PR1_CAPABILITIES = [
  "inference.status.v2",
  "inference.select.v1",
  "inference.disconnect.v1",
];
const KEY_HASH = "0123456789abcdef".repeat(4);
const SAVED_OPENROUTER = {
  route: "openrouter",
  provider: "openrouter",
  model: "openai/gpt-5",
};
const SAVED_CODEX = {
  route: "openai_codex",
  provider: "openai-codex",
  model: "gpt-5.3-codex",
};
const KEY_SAVED = {
  state: "key_saved",
  key_source: "agent",
  key_hash: KEY_HASH,
  hermes_key: "saved_key",
  other_pool_keys: "none",
};
const BASE = {
  telegram: { connected: false, home_channel: null, pending: [], approved: [] },
  google: { connected: false, email: null },
};

type InferenceOverrides = Record<string, unknown>;

function legacy(provider: string, model: string) {
  return parseConnectionsStatus({
    ...BASE,
    inference: {
      profile: provider === "openrouter" ? "openrouter" : "finite_private",
      provider,
      model,
    },
  });
}

function v2(
  overrides: InferenceOverrides = {},
  capabilities = PR1_CAPABILITIES
) {
  const { saved: savedOverride, routes, ...rest } = overrides;
  const saved = (savedOverride as typeof SAVED_OPENROUTER | undefined) ?? {
    route: "finite_private",
    provider: "custom",
    model: "glm-5-3-flash",
  };
  return parseConnectionsStatus({
    ...BASE,
    capabilities,
    inference: {
      profile: saved.route === "openrouter" ? "openrouter" : "finite_private",
      provider: saved.route === "openrouter" ? "openrouter" : "custom",
      model: saved.model,
      saved,
      routes: {
        finite_private: { state: "configured", reason: null },
        openrouter: {
          state: "no_key",
          key_source: null,
          key_hash: null,
          hermes_key: "none",
          other_pool_keys: "none",
        },
        ...(routes as object | undefined),
      },
      fallback: {
        state: "configured",
        reason: null,
        model: "glm-5-3-flash",
        extra_entries: 0,
      },
      operation: null,
      ...rest,
    },
  });
}

function operation(
  kind: "select" | "activate" | "disconnect",
  route: "finite_private" | "openrouter" | "openai_codex",
  state: "running" | "succeeded" | "failed",
  errorCode: string | null = null
) {
  return {
    id: `op_${"0a".repeat(16)}`,
    kind,
    route,
    model: kind === "disconnect" ? null : "openai/gpt-5",
    state,
    phase: kind === "disconnect" ? "cleanup" : "restarting",
    error_code: errorCode,
    attempts: 1,
    updated_at_ms: 1_790_000_000_000,
  };
}

function render(element: ReactElement) {
  return renderToStaticMarkup(element);
}

function text(markup: string) {
  return markup
    .replace(/<[^>]+>/gu, " ")
    .replace(/&#x27;/gu, "'")
    .replace(/&quot;/gu, '"')
    .replace(/&lt;/gu, "<")
    .replace(/&gt;/gu, ">")
    .replace(/&amp;/gu, "&")
    .replace(/\s+/gu, " ")
    .replace(/ ([.,:;)])/gu, "$1")
    .trim();
}

function panel(status: ReturnType<typeof parseConnectionsStatus> | null) {
  return render(
    createElement(InferenceConnections, {
      status,
      busy: false,
      send: async () => ({ ok: true as const, result: null }),
      refresh: async () => {},
    })
  );
}

test("legacy agents retain the one-call Save and expose no capability-gated controls", () => {
  const markup = panel(legacy("custom", "glm-5-3-flash"));
  assert.match(text(markup), /New conversations use Finite Private · glm-5-3-flash\./u);
  assert.match(text(markup), /Backup details aren't available on this agent yet\./u);
  assert.match(markup, /data-testid="inference-openrouter-open"/u);
  assert.doesNotMatch(
    markup,
    /inference-openrouter-(?:use|model-input|replace-key|disconnect|save-and-use|save-only)/u
  );
  assert.doesNotMatch(markup, /inference-operation-(?:line|retry)/u);

  const openRouter = panel(legacy("openrouter", "openai/gpt-5"));
  assert.match(text(openRouter), /OpenRouter Agent default/u);
  assert.match(openRouter, /data-testid="inference-openrouter-open"/u);

  const codex = panel(legacy("openai-codex", "gpt-5.5"));
  assert.match(text(codex), /New conversations use ChatGPT · gpt-5\.5\./u);
  assert.match(text(codex), /ChatGPT Agent default ChatGPT is this agent's default \(set in chat\)\./u);
});

test("advertised capabilities alone reveal connect, select, and disconnect controls", () => {
  const pr1 = panel(v2());
  assert.match(pr1, /data-testid="inference-openrouter-save"/u);
  assert.doesNotMatch(pr1, /inference-openrouter-save-and-use|inference-openrouter-save-only/u);

  const full = panel(v2({}, ALL_CAPABILITIES));
  assert.match(full, /data-testid="inference-openrouter-save-and-use"/u);
  assert.match(full, /data-testid="inference-openrouter-save-only"/u);

  const saved = panel(
    v2(
      { saved: SAVED_OPENROUTER, routes: { openrouter: KEY_SAVED } },
      ALL_CAPABILITIES
    )
  );
  for (const id of [
    "inference-openrouter-use",
    "inference-openrouter-replace-key",
    "inference-openrouter-disconnect",
  ]) {
    assert.match(saved, new RegExp(`data-testid="${id}"`, "u"), id);
  }
});

test("unknown facts stay neutral and never imply a repair or a confirmed key", () => {
  const markup = panel(
    v2({
      saved: SAVED_OPENROUTER,
      routes: {
        finite_private: { state: "unknown", reason: null },
        openrouter: {
          state: "unknown",
          key_source: null,
          key_hash: null,
          hermes_key: "unknown",
          other_pool_keys: "unknown",
        },
      },
      fallback: {
        state: "unknown",
        reason: null,
        model: null,
        extra_entries: 0,
      },
    })
  );
  assert.match(text(markup), /The agent couldn't confirm the Finite Private backup right now\./u);
  assert.match(text(markup), /Finite Private Not confirmed/u);
  assert.match(text(markup), /OpenRouter Not confirmed/u);
  assert.match(markup, /data-testid="inference-openrouter-key-input"/u);
  assert.doesNotMatch(
    markup,
    /is-attention|Needs attention|inference-openrouter-(?:use|replace-key|disconnect)/u
  );
});

test("backup copy distinguishes configured, unavailable, customized, and unchecked states", () => {
  const cases: Array<[InferenceOverrides, RegExp]> = [
    [
      { saved: SAVED_OPENROUTER },
      /Finite Private backup is configured\. If OpenRouter returns an error, Finite Private answers\./u,
    ],
    [
      {
        fallback: {
          state: "unavailable",
          reason: "credential_missing",
          model: null,
          extra_entries: 0,
        },
      },
      /Finite Private can't step in right now: this agent has no Finite Private credential\./u,
    ],
    [
      { fallback: { state: "custom", reason: null, model: null, extra_entries: 0 } },
      /Backup is customized in Hermes\./u,
    ],
    [
      { fallback: { state: "unknown", reason: null, model: null, extra_entries: 0 } },
      /The agent couldn't confirm the Finite Private backup right now\./u,
    ],
  ];
  for (const [status, expected] of cases) assert.match(text(panel(v2(status))), expected);
});

test("running and failed disconnects lock every competing change and name the reason", () => {
  for (const state of ["running", "failed"] as const) {
    const markup = panel(
      v2(
        {
          saved: SAVED_OPENROUTER,
          routes: { openrouter: KEY_SAVED },
          operation: operation(
            "disconnect",
            "openai_codex",
            state,
            state === "failed" ? "verify_failed" : null
          ),
        },
        ALL_CAPABILITIES
      )
    );
    const reasonId = markup.match(
      /<div id="([^"]+)" class="ocean-inference-summary__operation"/u
    )?.[1];
    assert.ok(reasonId);
    for (const id of [
      "inference-finite-private-use",
      "inference-openrouter-use",
      "inference-openrouter-replace-key",
      "inference-openrouter-disconnect",
    ]) {
      const button = markup.match(
        new RegExp(`<button[^>]*data-testid="${id}"[^>]*>`, "u")
      )?.[0];
      assert.ok(button, id);
      assert.match(button, / disabled=""/u, id);
      assert.match(button, new RegExp(`aria-describedby="${reasonId}"`, "u"), id);
    }
    const retry = markup.match(
      /<button[^>]*data-testid="inference-operation-retry"[^>]*>/u
    )?.[0];
    if (state === "running") {
      assert.equal(retry, undefined);
    } else {
      assert.ok(retry);
      assert.doesNotMatch(retry, / disabled=""/u);
    }
    assert.match(text(markup), /Hermes web dashboard is paused until this finishes/u);
  }

  for (const kind of ["select", "activate"] as const) {
    const markup = panel(
      v2({
        saved: SAVED_OPENROUTER,
        routes: { openrouter: KEY_SAVED },
        operation: operation(kind, "openrouter", "failed"),
      })
    );
    for (const id of [
      "inference-finite-private-use",
      "inference-openrouter-use",
      "inference-openrouter-replace-key",
      "inference-openrouter-disconnect",
    ]) {
      const button = markup.match(
        new RegExp(`<button[^>]*data-testid="${id}"[^>]*>`, "u")
      )?.[0];
      assert.ok(button, `${kind}: ${id}`);
      assert.doesNotMatch(button, / disabled=""|aria-describedby/u, `${kind}: ${id}`);
    }
  }
});

test("an expired polling window never unlocks a running mutation", () => {
  const view = inferenceView(
    v2({ operation: operation("disconnect", "openrouter", "running") })
  );
  assert.deepEqual(watchState(view, true, view.operation?.id ?? null), {
    delay: null,
    pollKey: view.operation?.id,
    pollExpired: true,
    locked: true,
  });
  assert.equal(watchState(view, false, null).locked, true);
});

test("agent error codes render bounded, actionable copy", () => {
  const view = inferenceView(v2());
  const cases: Array<[string | null, string, string]> = [
    ["not_connected", "ignored", "Connect OpenRouter first."],
    ["credential_rejected", "OpenRouter rejected it", "OpenRouter rejected it"],
    ["key_allowance_exhausted", "ignored", "This key has no remaining allowance. Raise its limit at openrouter.ai/keys, or use another key."],
    ["provider_unavailable", "ignored", "Couldn't reach OpenRouter. Nothing changed. Try again."],
    ["facts_unavailable", "ignored", "The agent couldn't check its setup right now. Try again in a moment."],
    ["activation_not_recorded", "ignored", "The key was saved, but the agent couldn't record the switch to OpenRouter."],
    ["agent_update_required", "ignored", "This agent needs an update for this."],
    [null, "safe server message", "safe server message"],
  ];
  for (const [code, message, expected] of cases) {
    const notice: InferenceNotice = {
      card: "openrouter",
      kind: "error",
      code,
      message,
      route: "openrouter",
    };
    assert.equal(
      text(render(createElement(NoticeLine, { notice, view, testId: "notice" }))),
      expected,
      code ?? "fallback"
    );
  }
});

test("OpenRouter key fields stay password-only and account links use only valid hashes", () => {
  const valid = render(
    createElement(OpenRouterConnection, {
      view: inferenceView(
        v2({ saved: SAVED_OPENROUTER, routes: { openrouter: KEY_SAVED } }, ALL_CAPABILITIES)
      ),
      busy: false,
      lock: { locked: false, reasonId: "operation" },
      run: async () => ({ ok: true as const, result: null }),
      notice: null,
    })
  );
  assert.match(valid, new RegExp(`https://openrouter.ai/keys/${KEY_HASH}`, "u"));
  assert.match(valid, new RegExp(`api_key_hash=${KEY_HASH}`, "u"));
  assert.match(valid, /target="_blank" rel="noopener noreferrer"/u);

  const form = panel(v2());
  const keyInput = form.match(/<input[^>]*data-testid="inference-openrouter-key-input"[^>]*>/u)?.[0];
  assert.ok(keyInput);
  assert.match(keyInput, /type="password"/u);
  assert.match(keyInput, /autoComplete="off"|autocomplete="off"/u);
  assert.doesNotMatch(form, /sk-or-|apiKey=/u);
});

test("key submission clears the credential and preserves use, store, and legacy intent", async () => {
  const events: unknown[] = [];
  const cases = [
    ["use", { action: "openrouter_connect_key", apiKey: "sk-or-test", activate: { model: "openai/gpt-5" } }],
    ["store", { action: "openrouter_connect_key", apiKey: "sk-or-test" }],
    ["legacy", { action: "inference", profile: "openrouter", apiKey: "sk-or-test", model: "openai/gpt-5" }],
  ] as const;
  for (const [kind, expected] of cases) {
    events.length = 0;
    await submitOpenRouterKey(
      " sk-or-test ",
      kind,
      " openai/gpt-5 ",
      () => events.push("cleared"),
      async (action) => {
        events.push(action);
      }
    );
    assert.deepEqual(events, ["cleared", expected], kind);
  }

  events.length = 0;
  await assert.rejects(
    submitOpenRouterKey(
      "sk-or-test",
      "use",
      "openai/gpt-5",
      () => events.push("cleared"),
      async () => {
        events.push("sent");
        throw new Error("offline");
      }
    )
  );
  assert.deepEqual(events, ["cleared", "sent"]);
});

test("reconnect clears a stale removal while failed replacement and environment keys stay visible", () => {
  const removed = (openrouter: Record<string, unknown>) =>
    render(
      createElement(OpenRouterRemoved, {
        view: inferenceView(v2({ routes: { openrouter } })),
        keyHash: KEY_HASH,
      })
    );

  assert.match(removed({
    state: "no_key",
    key_source: null,
    key_hash: null,
    hermes_key: "none",
    other_pool_keys: "none",
  }), /OpenRouter was removed from this agent/u);
  assert.equal(removed(KEY_SAVED), "");
  assert.match(
    text(removed({ ...KEY_SAVED, key_source: "environment", key_hash: null })),
    /still gets an OpenRouter key from its environment/u
  );
});

test("ChatGPT stays read-only and reports only saved or advertised facts", () => {
  const hidden = panel(v2({ saved: SAVED_OPENROUTER }));
  assert.doesNotMatch(hidden, /data-testid="inference-codex"/u);

  const signedIn = panel(
    v2(
      {
        saved: SAVED_CODEX,
        routes: {
          openai_codex: {
            state: "signed_in",
            quota_reset_at_ms: null,
            reported_quota_reset_at_ms: null,
            login: null,
          },
        },
      },
      ALL_CAPABILITIES
    )
  );
  assert.match(text(signedIn), /ChatGPT Signed in Signed in to ChatGPT\./u);
  const card = signedIn.match(
    /<section class="ocean-connection-card" data-testid="inference-codex">[\s\S]*?<\/section>/u
  )?.[0];
  assert.ok(card);
  assert.doesNotMatch(card, /<button/u);
});
