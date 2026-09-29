import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { codexCardState, CodexConnection } from "@/components/codex-connection";
import { ConnectionsPanel } from "@/components/connections-panel";
import {
  backupLine,
  commandErrorText,
  FinitePrivateCard,
  finitePrivateAction,
  checkAgain,
  controlsLocked,
  InferenceConnections,
  NoticeLine,
  OperationLine,
  operationText,
  POLL_LIMIT_MS,
  watchState,
  STORAGE_ONLY_COPY,
  type InferenceNotice,
  type InferenceRun,
} from "@/components/inference-connections";
import {
  OPENROUTER_DISCONNECT_TITLE,
  OpenRouterConnection,
  OpenRouterKeyForm,
  openRouterDisconnectCopy,
  openRouterKeyAction,
  openRouterLinks,
  OpenRouterRemoved,
  openRouterUseAction,
  submitOpenRouterKey,
} from "@/components/openrouter-connection";
import { parseConnectionsStatus } from "@/lib/hosted-agent-controls";
import { inferenceView, pollDelayMs } from "@/lib/inference-status";

// Every string a test renders is collected here and scanned for forbidden claims at the end.
const rendered: string[] = [];

function html(element: ReactElement) {
  const markup = renderToStaticMarkup(element);
  rendered.push(text(markup));
  return markup;
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

function copy<T extends string | null | undefined>(value: T) {
  if (value) rendered.push(value);
  return value;
}

const ALL = [
  "inference.status.v2", "inference.select.v1", "inference.disconnect.v1",
  "openrouter.connect.v1", "openrouter.usage.v1", "codex.login.v1", "codex.models.v1",
];
const PR1 = ["inference.status.v2", "inference.select.v1", "inference.disconnect.v1"];
const KEY_HASH = "0123456789abcdef".repeat(4);
const OP_ID = `op_${"0a".repeat(16)}`;

const LEGACY_BASE = {
  telegram: { connected: false, home_channel: null, pending: [], approved: [] },
  google: { connected: false, email: null },
};

function legacy(provider: string, model: string) {
  return parseConnectionsStatus({
    ...LEGACY_BASE,
    inference: { profile: provider === "openrouter" ? "openrouter" : "finite_private", provider, model },
  });
}

type Inference = Record<string, unknown>;
function v2(inference: Inference = {}, capabilities: string[] = PR1) {
  const { saved: savedOverride, routes, ...rest } = inference;
  const saved = (savedOverride as { route: string; model: string } | undefined) ?? {
    route: "finite_private", provider: "custom", model: "glm-5-3-flash",
  };
  return parseConnectionsStatus({
    ...LEGACY_BASE,
    capabilities,
    inference: {
      profile: saved.route === "openrouter" ? "openrouter" : "finite_private",
      provider: saved.route === "openrouter" ? "openrouter" : "custom",
      model: saved.model,
      saved,
      routes: {
        finite_private: { state: "configured", reason: null },
        openrouter: { state: "no_key", key_source: null, key_hash: null, hermes_key: "none", other_pool_keys: "none" },
        ...(routes as object | undefined),
      },
      fallback: { state: "configured", reason: null, model: "glm-5-3-flash", extra_entries: 0 },
      operation: null,
      ...rest,
    },
  });
}

const view = (status: ReturnType<typeof parseConnectionsStatus>) => inferenceView(status);
const SAVED_OR = { route: "openrouter", provider: "openrouter", model: "openai/gpt-5" };
const SAVED_CODEX = { route: "openai_codex", provider: "openai-codex", model: "gpt-5.3-codex" };
const KEY_SAVED = { state: "key_saved", key_source: "agent", key_hash: KEY_HASH, hermes_key: "saved_key", other_pool_keys: "none" };
const noRun: InferenceRun = async () => ({ ok: true, result: null });
const UNLOCKED = { locked: false, reasonId: "operation-line" };
const LOCKED = { locked: true, reasonId: "operation-line" };

function operation(kind: string, route: string, state: string, extra: Inference = {}) {
  return {
    id: OP_ID, kind, route, model: kind === "disconnect" ? null : "openai/gpt-5", state,
    phase: kind === "disconnect" ? "cleanup" : "restarting", error_code: null, attempts: 1,
    updated_at_ms: 1_790_000_000_000, ...extra,
  };
}

function fallback(state: string, reason: string | null = null, extra = 0) {
  return { state, reason, model: state === "configured" ? "glm-5-3-flash" : null, extra_entries: extra };
}

function panel(status: ReturnType<typeof parseConnectionsStatus> | null) {
  return html(createElement(InferenceConnections, { status, busy: false, send: async () => ({ ok: true as const, result: null }), refresh: async () => {} }));
}

test("every backup line, and only a configured backup says Finite Private answers", () => {
  const cases: Array<[Inference, string]> = [
    [{ saved: SAVED_OR }, "Finite Private backup is configured. If OpenRouter returns an error, Finite Private answers."],
    [{ saved: SAVED_CODEX }, "Finite Private backup is configured. If ChatGPT returns an error, Finite Private answers."],
    [{}, "Finite Private backup is configured for conversations that use another model."],
    [{ saved: SAVED_OR, fallback: fallback("configured", null, 2) }, "Finite Private backup is configured. If OpenRouter returns an error, Finite Private answers. Your other backup models are tried after it."],
    [{ fallback: fallback("configured", null, 1) }, "Finite Private backup is configured for conversations that use another model. Your other backup models are tried after it."],
    [{ fallback: fallback("unavailable", "settings_missing") }, "Finite Private can't step in right now: Finite Private isn't set up on this agent."],
    [{ fallback: fallback("unavailable", "credential_missing") }, "Finite Private can't step in right now: this agent has no Finite Private credential."],
    [{ fallback: fallback("unavailable", "stale_config") }, "Finite Private can't step in right now: it updates the next time the agent restarts."],
    [{ fallback: fallback("unavailable") }, "Finite Private can't step in right now."],
    [{ fallback: fallback("not_configured") }, "Finite Private backup isn't set up on this agent."],
    [{ fallback: fallback("not_configured", "credential_missing") }, "Finite Private backup isn't set up on this agent: this agent has no Finite Private credential."],
    [{ fallback: fallback("off") }, "Backup to Finite Private is turned off in Hermes."],
    [{ fallback: fallback("custom") }, "Backup is customized in Hermes."],
    // A v2 agent that reports `unknown` couldn't confirm the backup; it isn't too old.
    [{ fallback: fallback("unknown") }, "The agent couldn't confirm the Finite Private backup right now."],
    [{ saved: SAVED_OR, fallback: fallback("unknown") }, "The agent couldn't confirm the Finite Private backup right now."],
  ];
  for (const [inference, expected] of cases) {
    const line = copy(backupLine(view(v2(inference))));
    assert.equal(line, expected);
    if (/answers/u.test(line)) {
      assert.ok(inference.fallback === undefined || (inference.fallback as { state: string }).state === "configured", line);
    }
  }
  assert.equal(copy(backupLine(view(legacy("openrouter", "openai/gpt-5")))), "Backup details aren't available on this agent yet.");
  assert.equal(copy(backupLine(view(v2({ saved: SAVED_OR }, ["inference.select.v1"])))), "Backup details aren't available on this agent yet.");
});

test("the Finite Private card states", () => {
  const card = (status: ReturnType<typeof parseConnectionsStatus>) =>
    text(html(createElement(FinitePrivateCard, { view: view(status), busy: false, lock: UNLOCKED, run: noRun, notice: null })));
  const configured = card(v2());
  assert.match(configured, /Finite Private Configured glm-5-3-flash/u);
  assert.doesNotMatch(configured, /Use Finite Private/u);
  assert.match(card(v2({ routes: { finite_private: { state: "not_configured", reason: "settings_missing" } } })), /Needs attention .*Finite Private isn't set up on this agent\./u);
  assert.match(card(v2({ routes: { finite_private: { state: "not_configured", reason: "credential_missing" } } })), /Needs attention .*This agent has no Finite Private credential\./u);
  assert.match(card(legacy("custom", "glm-5-3-flash")), /Agent default glm-5-3-flash/u);
  assert.match(card(legacy("openrouter", "openai/gpt-5")), /Not the agent default/u);
});

test("a v2 agent that couldn't confirm Finite Private: \"Not confirmed\", in the neutral style, with no change of controls", () => {
  // Unknown facts do not establish that the owner needs to repair anything.
  const unknown = { finite_private: { state: "unknown", reason: null } };
  const cardMarkup = (status: ReturnType<typeof parseConnectionsStatus>) =>
    html(createElement(FinitePrivateCard, { view: view(status), busy: false, lock: UNLOCKED, run: noRun, notice: null }));
  const saved = cardMarkup(v2({ routes: unknown }));
  assert.equal(
    text(saved),
    "Finite Private Not confirmed glm-5-3-flash Finite's own private model service. The agent couldn't confirm its Finite Private setup right now."
  );
  assert.match(saved, /ocean-connection-card__status is-disconnected/u);
  assert.doesNotMatch(saved, /is-attention|text-destructive|Needs attention/u);
  assert.doesNotMatch(saved, /inference-finite-private-use/u);
  // Another saved route keeps "Use Finite Private", enabled, as for any other state.
  const other = cardMarkup(v2({ saved: SAVED_OR, routes: unknown }));
  assert.match(text(other), /Finite Private Not confirmed Finite's own private model service\. The agent couldn't confirm its Finite Private setup right now\. Use Finite Private/u);
  assert.doesNotMatch(other.match(/<button[^>]*inference-finite-private-use[^>]*>/u)?.[0] ?? "", / disabled=""/u);
  // Known facts keep their states.
  assert.match(cardMarkup(v2({ routes: { finite_private: { state: "not_configured", reason: null } } })), /is-attention/u);
  assert.doesNotMatch(text(cardMarkup(v2())), /Not confirmed|couldn't confirm/u);
  // Without inference.status.v2 the agent reports no facts, so there is nothing to confirm.
  for (const status of [legacy("custom", "glm-5-3-flash"), legacy("openrouter", "openai/gpt-5")]) {
    assert.doesNotMatch(text(cardMarkup(status)), /Not confirmed|couldn't confirm/u);
  }
});

test("\"Use Finite Private\" appears whenever the saved route isn't Finite Private", () => {
  for (const status of [
    v2({ saved: SAVED_OR }),
    v2({ saved: SAVED_CODEX }),
    v2({ saved: { route: "other", provider: "anthropic", model: "claude" } }),
    legacy("openrouter", "openai/gpt-5"),
    legacy("openai-codex", "gpt-5.5"),
    legacy("anthropic", "claude"),
  ]) {
    const markup = html(createElement(FinitePrivateCard, { view: view(status), busy: false, lock: UNLOCKED, run: noRun, notice: null }));
    assert.match(markup, /data-testid="inference-finite-private-use"/u);
    assert.doesNotMatch(markup.match(/<button[^>]*inference-finite-private-use[^>]*>/u)?.[0] ?? "", / disabled=""/u);
  }
  const busy = html(createElement(FinitePrivateCard, { view: view(v2({ saved: SAVED_OR })), busy: true, lock: UNLOCKED, run: noRun, notice: null }));
  assert.match(busy.match(/<button[^>]*inference-finite-private-use[^>]*>/u)?.[0] ?? "", / disabled=""/u);
  assert.deepEqual(finitePrivateAction(view(v2({ saved: SAVED_OR }))), { action: "inference_select", route: "finite_private" });
  assert.deepEqual(finitePrivateAction(view(legacy("openrouter", "openai/gpt-5"))), { action: "inference", profile: "finite_private" });
});

function openRouterCard(status: ReturnType<typeof parseConnectionsStatus>, notice: InferenceNotice | null = null) {
  return html(createElement(OpenRouterConnection, { view: view(status), busy: false, lock: UNLOCKED, run: noRun, notice }));
}

test("OpenRouter without a key: paste through connect when advertised, else the legacy one-call Save", () => {
  const pr1 = openRouterCard(v2());
  assert.match(text(pr1), /OpenRouter Not connected Use your own OpenRouter account\. Finite Private is configured as a backup\./u);
  assert.match(pr1, /data-testid="inference-openrouter-key-input"/u);
  // without openrouter.connect.v1, Save goes through v1, which always makes OpenRouter the saved default.
  assert.match(pr1, /data-testid="inference-openrouter-save"[^>]*>Save and use OpenRouter</u);
  assert.match(text(pr1), /Save and use OpenRouter This also makes OpenRouter this agent's default\./u);
  assert.doesNotMatch(pr1, /inference-openrouter-save-only|inference-openrouter-disconnect"|inference-openrouter-links/u);

  const pr2 = openRouterCard(v2({}, ALL));
  assert.match(pr2, /data-testid="inference-openrouter-save-and-use"/u);
  assert.match(pr2, /data-testid="inference-openrouter-save-only"/u);
  assert.doesNotMatch(pr2, /data-testid="inference-openrouter-save"/u);
  assert.doesNotMatch(text(pr2), /Save and use OpenRouter|This also makes OpenRouter/u);

  const noBackup = openRouterCard(v2({ fallback: fallback("off") }));
  assert.doesNotMatch(text(noBackup), /configured as a backup/u);

  assert.deepEqual(openRouterKeyAction("use", " sk-or-v1-fake ", "openai/gpt-5"), {
    action: "openrouter_connect_key", apiKey: "sk-or-v1-fake", activate: { model: "openai/gpt-5" },
  });
  assert.deepEqual(openRouterKeyAction("store", "sk-or-v1-fake", "openai/gpt-5"), { action: "openrouter_connect_key", apiKey: "sk-or-v1-fake" });
  assert.deepEqual(openRouterKeyAction("legacy", "sk-or-v1-fake", "openai/gpt-5"), {
    action: "inference", profile: "openrouter", apiKey: "sk-or-v1-fake", model: "openai/gpt-5",
  });
  assert.deepEqual(openRouterKeyAction("legacy", "", "openai/gpt-5"), { action: "inference", profile: "openrouter", model: "openai/gpt-5" });
});

test("the key form without openrouter.connect.v1 says it also makes OpenRouter the default, for a first key and a replaced one", () => {
  const form = (connect: boolean, keyRequired: boolean, onCancel: (() => void) | null, openRouterSaved = false) =>
    html(createElement(OpenRouterKeyForm, {
      keyRequired, keyPlaceholder: "OpenRouter API key", connect, openRouterSaved, model: "anthropic/claude-sonnet-4.6",
      setModel: () => {}, busy: false, onSubmit: async () => {}, onCancel,
    }));
  const line = "This also makes OpenRouter this agent's default.";
  // First key on a PR1 agent (no Cancel), "Replace key" on a PR1 agent (Cancel), and today's agentd (optional key).
  for (const [name, markup] of [
    ["PR1 first key", form(false, true, null)],
    ["PR1 Replace key", form(false, true, () => {})],
    ["today's agentd", form(false, false, () => {})],
  ] as const) {
    assert.match(markup, /data-testid="inference-openrouter-save"[^>]*>Save and use OpenRouter</u, name);
    assert.match(markup, /data-testid="inference-openrouter-key-default-line"/u, name);
    assert.ok(text(markup).includes(line), name);
    assert.doesNotMatch(markup, /inference-openrouter-save-and-use|inference-openrouter-save-only/u, name);
  }
  // when OpenRouter is the saved default already, the button text alone says it; the line is absent.
  for (const [name, markup] of [
    ["PR1 Replace key, OpenRouter saved", form(false, true, () => {}, true)],
    ["today's agentd, OpenRouter saved", form(false, false, () => {}, true)],
  ] as const) {
    assert.match(markup, /data-testid="inference-openrouter-save"[^>]*>Save and use OpenRouter</u, name);
    assert.doesNotMatch(markup, /inference-openrouter-key-default-line|This also makes OpenRouter/u, name);
  }
  // With openrouter.connect.v1 the form is as designs it: "Save and use" and "Save only", and no extra line.
  for (const onCancel of [null, () => {}]) {
    const markup = form(true, true, onCancel);
    assert.match(markup, /data-testid="inference-openrouter-save-and-use"[^>]*>Save and use</u);
    assert.match(markup, /data-testid="inference-openrouter-save-only"[^>]*>Save only</u);
    assert.doesNotMatch(markup, /inference-openrouter-key-default-line|Save and use OpenRouter/u);
  }
  // The command is unchanged: the one-call v1 apply with the key and the model.
  assert.deepEqual(openRouterKeyAction("legacy", "sk-or-v1-fake", "openai/gpt-5"), {
    action: "inference", profile: "openrouter", apiKey: "sk-or-v1-fake", model: "openai/gpt-5",
  });
});

const OR_UNKNOWN = { state: "unknown", key_source: null, key_hash: null, hermes_key: "unknown", other_pool_keys: "unknown" };
const OR_NO_KEY = { state: "no_key", key_source: null, key_hash: null, hermes_key: "none", other_pool_keys: "none" };
const formOf = (markup: string) => markup.match(/<form[\s\S]*<\/form>/u)?.[0] ?? "";

test("OpenRouter unknown on a v2 agent: \"Not confirmed\", neutral, a note, the key form, and no action of a saved key", () => {
  const unknown = openRouterCard(v2({ routes: { openrouter: OR_UNKNOWN } }));
  assert.match(
    text(unknown),
    /^OpenRouter Not confirmed Use your own OpenRouter account\. Finite Private is configured as a backup\. The agent couldn't confirm its OpenRouter setup right now\./u
  );
  assert.match(unknown, /ocean-connection-card__status is-disconnected" data-testid="inference-openrouter-state"/u);
  assert.match(unknown, /data-testid="inference-openrouter-note"/u);
  assert.doesNotMatch(unknown, /is-attention|text-destructive|Status unavailable/u);
  // The key form, as with no key saved: no Cancel, since there is nothing else to show.
  assert.match(unknown, /data-testid="inference-openrouter-key-input"/u);
  assert.match(unknown, /data-testid="inference-openrouter-save"[^>]*>Save and use OpenRouter</u);
  assert.match(text(unknown), /This also makes OpenRouter this agent's default\./u);
  assert.doesNotMatch(unknown, /inference-openrouter-cancel-key/u);
  // Removed: the controls of a saved key, which the daemon refuses when there is no key.
  for (const id of ["inference-openrouter-use", "inference-openrouter-replace-key", "inference-openrouter-disconnect", "inference-openrouter-model-input"]) {
    assert.doesNotMatch(unknown, new RegExp(`data-testid="${id}"`, "u"), id);
  }

  // The form is the no_key form exactly, so it sends the same command: the v1 apply on PR1, connect on a later agent.
  for (const capabilities of [PR1, ALL]) {
    const unknownForm = formOf(openRouterCard(v2({ routes: { openrouter: OR_UNKNOWN } }, capabilities)));
    const noKeyForm = formOf(openRouterCard(v2({ routes: { openrouter: OR_NO_KEY } }, capabilities)));
    assert.ok(unknownForm.length > 0);
    assert.equal(unknownForm, noKeyForm, capabilities.join(","));
  }

  // OpenRouter saved and unknown: the same form without the default-change explanation, the links, and still no saved-key action.
  const saved = openRouterCard(v2({ saved: SAVED_OR, routes: { openrouter: OR_UNKNOWN } }));
  assert.match(text(saved), /^OpenRouter Not confirmed /u);
  assert.match(saved, /data-testid="inference-openrouter-save"[^>]*>Save and use OpenRouter</u);
  assert.doesNotMatch(saved, /inference-openrouter-key-default-line/u);
  assert.match(saved, /data-testid="inference-openrouter-links"/u);
  for (const id of ["inference-openrouter-use", "inference-openrouter-replace-key", "inference-openrouter-disconnect"]) {
    assert.doesNotMatch(saved, new RegExp(`data-testid="${id}"`, "u"), id);
  }

  // The lock rule is unchanged: a running operation disables this form's Save and names the reason.
  const locked = panel(v2({ routes: { openrouter: OR_UNKNOWN }, operation: operation("select", "finite_private", "running", { model: null }) }));
  const { reasonId, button } = lockedButtons(locked);
  assert.match(button("inference-openrouter-save"), / disabled=""/u);
  assert.match(button("inference-openrouter-save"), new RegExp(`aria-describedby="${reasonId}"`, "u"));

  // Known states and agents without inference.status.v2 are unchanged.
  assert.match(text(openRouterCard(v2())), /^OpenRouter Not connected /u);
  assert.match(text(openRouterCard(v2({ routes: { openrouter: KEY_SAVED } }))), /^OpenRouter Key saved /u);
  for (const status of [legacy("custom", "glm-5-3-flash"), legacy("openrouter", "openai/gpt-5")]) {
    assert.doesNotMatch(text(openRouterCard(status)), /Not confirmed|couldn't confirm/u);
  }
});

test("OpenRouter with a saved key: account line by source, extra lines, actions, links", () => {
  const accounts: Array<[string, string]> = [
    ["agent", "Key saved in this agent"],
    ["legacy_config", "Key saved in an older format. Select Use OpenRouter to finish setting it up."],
    ["environment", "Key provided by this agent's environment"],
  ];
  for (const [source, line] of accounts) {
    const markup = openRouterCard(v2({ routes: { openrouter: { ...KEY_SAVED, key_source: source } } }));
    assert.match(text(markup), new RegExp(`OpenRouter Key saved ${line.replace(/[.']/gu, ".")}`, "u"));
  }
  const saved = openRouterCard(v2({ saved: SAVED_OR, routes: { openrouter: { ...KEY_SAVED, hermes_key: "other_key", other_pool_keys: "present" } } }));
  assert.match(text(saved), /Hermes is set up to use a different OpenRouter key than the one saved here\./u);
  assert.match(text(saved), /Hermes also has other OpenRouter keys, added in chat or the terminal, and may use them\./u);
  assert.match(saved, /data-testid="inference-openrouter-model-input"[^>]*value="openai\/gpt-5"|value="openai\/gpt-5"[^>]*data-testid="inference-openrouter-model-input"/u);
  assert.match(saved, /data-testid="inference-openrouter-use"[^>]*>Save model</u);
  assert.match(saved, /data-testid="inference-openrouter-replace-key"/u);
  assert.match(saved, /data-testid="inference-openrouter-disconnect"/u);
  assert.doesNotMatch(saved, /inference-openrouter-key-input/u);

  const notSaved = openRouterCard(v2({ routes: { openrouter: KEY_SAVED } }));
  assert.match(notSaved, /data-testid="inference-openrouter-use"[^>]*>Use OpenRouter</u);
  assert.match(notSaved, /value="anthropic\/claude-sonnet-4.6"/u);
  assert.doesNotMatch(text(notSaved), /different OpenRouter key|other OpenRouter keys/u);

  const noDisconnect = openRouterCard(v2({ routes: { openrouter: KEY_SAVED } }, ["inference.status.v2", "inference.select.v1"]));
  assert.doesNotMatch(noDisconnect, /data-testid="inference-openrouter-disconnect"/u);
  assert.match(text(noDisconnect), /This agent needs an update to disconnect here\./u);

  assert.deepEqual(openRouterUseAction(view(v2({ routes: { openrouter: KEY_SAVED } })), " openai/gpt-5 "), {
    action: "inference_select", route: "openrouter", model: "openai/gpt-5",
  });
});

test("OpenRouter links use constants plus a validated key hash, and open safely", () => {
  const markup = openRouterCard(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED } }));
  const links = [...markup.matchAll(/<a\b[^>]*>/gu)].map(([tag]) => tag);
  assert.deepEqual(
    links.map((tag) => tag.match(/href="([^"]*)"/u)?.[1]),
    [
      "https://openrouter.ai/settings/credits",
      "https://openrouter.ai/settings/credits",
      `https://openrouter.ai/keys/${KEY_HASH}`,
      `https://openrouter.ai/logs?api_key_hash=${KEY_HASH}`,
    ]
  );
  for (const tag of links) {
    assert.match(tag, /target="_blank"/u);
    assert.match(tag, /rel="noopener noreferrer"/u);
  }
  assert.match(text(markup), /Add funds Account credits Your account balance is shown on OpenRouter\. Manage this key Key activity Key links only work when you're signed in to the OpenRouter account that owns the key\./u);
  for (const hash of [null, "ABC", `${KEY_HASH}/../x`]) {
    assert.deepEqual(openRouterLinks(hash).map((link) => link.href), [
      "https://openrouter.ai/settings/credits",
      "https://openrouter.ai/settings/credits",
      "https://openrouter.ai/settings/keys",
      "https://openrouter.ai/activity",
    ]);
  }
  // Links show for a saved key or a saved route, and not otherwise.
  assert.match(openRouterCard(legacy("openrouter", "openai/gpt-5")), /inference-openrouter-links/u);
  assert.doesNotMatch(openRouterCard(legacy("custom", "glm-5-3-flash")), /inference-openrouter-links/u);
});

test("an agent running today's agentd keeps today's controls and nothing that needs a capability", () => {
  for (const status of [legacy("custom", "glm-5-3-flash"), legacy("openrouter", "openai/gpt-5"), legacy("openai-codex", "gpt-5.5")]) {
    const markup = panel(status);
    for (const hidden of [
      "inference-openrouter-disconnect\"", "inference-openrouter-save-only", "inference-openrouter-save-and-use",
      "inference-openrouter-replace-key", "inference-openrouter-model-input", "inference-operation-line",
      "inference-operation-retry",
    ]) {
      assert.equal(markup.includes(hidden), false, hidden);
    }
    assert.match(markup, /data-testid="inference-openrouter-open"/u);
    // no ChatGPT card unless ChatGPT is the saved route, and never an update request.
    assert.equal(markup.includes('data-testid="inference-codex"'), status.inference.provider === "openai-codex");
    assert.doesNotMatch(text(markup), /Update needed|needs an update to connect ChatGPT/u);
    assert.match(text(markup), /Backup details aren't available on this agent yet\./u);
    assert.equal(pollDelayMs(view(status), true), null);
    const legacyView = view(status);
    assert.equal(openRouterUseAction(legacyView, "openai/gpt-5").action, "inference");
    assert.equal(finitePrivateAction(legacyView).action, "inference");
  }
  assert.match(text(panel(legacy("openai-codex", "gpt-5.5"))), /New conversations use ChatGPT · gpt-5\.5\. .*ChatGPT Agent default ChatGPT is this agent's default \(set in chat\)\./u);
  assert.match(text(panel(legacy("custom", "glm-5-3-flash"))), /New conversations use Finite Private · glm-5-3-flash\./u);
  assert.match(text(panel(legacy("anthropic", "claude"))), /New conversations use Custom model · claude, set in Hermes\. Finite can't manage this provider here\./u);
  // The legacy key form keeps today's optional key and one Save.
  const form = html(createElement(OpenRouterKeyForm, {
    keyRequired: false, keyPlaceholder: "Your key (optional)", connect: false, openRouterSaved: false, model: "anthropic/claude-sonnet-4.6",
    setModel: () => {}, busy: false, onSubmit: async () => {}, onCancel: () => {},
  }));
  assert.match(form, /placeholder="Your key \(optional\)"/u);
  assert.match(form, /data-testid="inference-openrouter-save"[^>]*>Save and use OpenRouter</u);
  assert.match(text(form), /This also makes OpenRouter this agent's default\./u);
  assert.doesNotMatch(form.match(/<button[^>]*inference-openrouter-save"[^>]*>/u)?.[0] ?? "", / disabled=""/u);
});

test("garbage or unknown inference facts never throw and never blank the other cards", () => {
  const garbage = parseConnectionsStatus({
    ...LEGACY_BASE,
    telegram: { connected: true, home_channel: "Owner", pending: [], approved: [] },
    capabilities: PR1,
    inference: {
      profile: "who-knows", provider: "custom", model: "glm-5-3-flash",
      saved: "nope", routes: { finite_private: 7, openrouter: { state: "works" }, openai_codex: [] },
      fallback: { state: "answers-everything" }, operation: { id: "op_bad" },
    },
  });
  assert.equal(garbage.telegram.connected, true);
  const markup = text(panel(garbage));
  assert.match(markup, /New conversations use Finite Private · glm-5-3-flash\./u);
  // A v2 agent's unreadable facts read as not confirmed, not as an agent too old to say.
  assert.match(markup, /The agent couldn't confirm the Finite Private backup right now\./u);
  assert.match(markup, /Finite Private Not confirmed/u);
  const unknown = v2({
    routes: {
      finite_private: { state: "unknown", reason: null },
      openrouter: { state: "unknown", key_source: null, key_hash: null, hermes_key: "unknown", other_pool_keys: "unknown" },
    },
    fallback: fallback("unknown"),
  });
  const unknownPanel = panel(unknown);
  // Unknown OpenRouter facts must not offer controls that assume a saved key.
  assert.match(text(unknownPanel), /OpenRouter Not confirmed Use your own OpenRouter account\. The agent couldn't confirm its OpenRouter setup right now\./u);
  // the summary and the Finite Private card say the agent couldn't confirm, and nothing asks for a repair.
  assert.match(text(unknownPanel), /The agent couldn't confirm the Finite Private backup right now\./u);
  assert.match(text(unknownPanel), /Finite Private Not confirmed glm-5-3-flash Finite's own private model service\. The agent couldn't confirm its Finite Private setup right now\./u);
  assert.doesNotMatch(unknownPanel, /is-attention|Needs attention|Backup details aren't available/u);
});

test("the OpenRouter disconnect dialog states what it does and doesn't guarantee", () => {
  const base =
    "Finite deletes the key saved in this agent and the agent's other copies, and restarts the agent's model service. " +
    "Programs the agent started earlier may keep a copy until they stop. " +
    "The key keeps working in your OpenRouter account until you revoke it there.";
  const modelClause = " Conversations that picked OpenRouter with /model will use the agent default.";
  assert.equal(OPENROUTER_DISCONNECT_TITLE, "Remove OpenRouter from this agent?");
  assert.equal(
    copy(openRouterDisconnectCopy(view(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED } })))),
    `${base} This agent will switch to Finite Private.${modelClause}`
  );
  assert.equal(copy(openRouterDisconnectCopy(view(v2({ routes: { openrouter: KEY_SAVED } })))), `${base}${modelClause}`);
});

test("after a disconnect, the revoke link uses the captured key, and an environment key is called out", () => {
  const removed = html(createElement(OpenRouterRemoved, { view: view(v2()), keyHash: KEY_HASH }));
  assert.match(text(removed), /^OpenRouter was removed from this agent\. Revoke the key in OpenRouter$/u);
  assert.match(removed, new RegExp(`href="https://openrouter.ai/keys/${KEY_HASH}"[^>]*target="_blank"[^>]*rel="noopener noreferrer"`, "u"));
  const generic = html(createElement(OpenRouterRemoved, { view: view(v2()), keyHash: null }));
  assert.match(generic, /href="https:\/\/openrouter.ai\/settings\/keys"/u);
  const environment = html(createElement(OpenRouterRemoved, {
    view: view(v2({ routes: { openrouter: { ...KEY_SAVED, key_source: "environment", key_hash: null } } })),
    keyHash: KEY_HASH,
  }));
  assert.match(text(environment), /This agent still gets an OpenRouter key from its environment, so OpenRouter can still be used\./u);
});

function operationLine(op: Inference, options: { showDone?: boolean; pollExpired?: boolean; retry?: boolean } = {}) {
  return html(createElement(OperationLine, {
    view: view(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED }, operation: op })),
    showDone: options.showDone ?? false,
    pollExpired: options.pollExpired ?? false,
    busy: false,
    onRetry: options.retry ? () => {} : null,
    onCheckAgain: () => {},
  }));
}

test("the operation line in every state", () => {
  const paused = " The agent's Hermes web dashboard is paused until this finishes.";
  const cases: Array<[Inference, string]> = [
    [operation("select", "openrouter", "running"), "Switching to OpenRouter · openai/gpt-5…"],
    [operation("activate", "openrouter", "running"), "Switching to OpenRouter · openai/gpt-5…"],
    [operation("select", "finite_private", "running", { model: null }), "Switching to Finite Private…"],
    [operation("disconnect", "openrouter", "running"), `Disconnecting OpenRouter… This connection may still be in use until this finishes.${paused}`],
    [operation("disconnect", "openrouter", "failed", { error_code: "verify_failed" }), `Disconnecting OpenRouter didn't finish. It may still be in use.${paused}`],
    [operation("select", "openrouter", "failed"), "Switching to OpenRouter · openai/gpt-5 didn't finish. Try again."],
    [operation("select", "openrouter", "failed", { error_code: "config_conflict" }), "Switching to OpenRouter · openai/gpt-5 didn't finish (settings changed while saving, maybe from chat). Try again."],
    [operation("activate", "openrouter", "failed", { error_code: "supervisor_unavailable" }), "Switching to OpenRouter · openai/gpt-5 didn't finish (the agent didn't restart cleanly). Try again."],
    [operation("select", "openrouter", "failed", { error_code: "helper_unavailable" }), "Switching to OpenRouter · openai/gpt-5 didn't finish (the agent couldn't finish cleaning up). Try again."],
    [operation("select", "openrouter", "failed", { error_code: "verify_failed" }), "Switching to OpenRouter · openai/gpt-5 didn't finish (the change didn't stick). Try again."],
    [operation("select", "openrouter", "failed", { error_code: "config_invalid" }), "Switching to OpenRouter · openai/gpt-5 didn't finish (the change isn't valid on this agent). Try again."],
  ];
  for (const [op, expected] of cases) {
    const markup = operationLine(op);
    assert.match(markup, /role="status" aria-live="polite"/u);
    assert.equal(text(markup.match(/<span data-testid="inference-operation-line">(.*?)<\/span>/u)?.[1] ?? ""), expected);
  }
  assert.equal(text(operationLine(operation("select", "openrouter", "succeeded"), { showDone: true })), "Done.");
  assert.equal(text(operationLine(operation("select", "openrouter", "succeeded"))), "");
  assert.match(operationLine(operation("disconnect", "openrouter", "failed"), { retry: true }), /data-testid="inference-operation-retry"[^>]*>Try again</u);
  assert.match(operationLine(operation("select", "openrouter", "running"), { pollExpired: true }), /data-testid="inference-operation-check-again"/u);
  assert.doesNotMatch(operationLine(operation("select", "openrouter", "running")), /check-again/u);
  const running = view(v2({ operation: operation("select", "openrouter", "running") }));
  assert.equal(pollDelayMs(running, true), 3000);
  assert.equal(pollDelayMs(running, false), null);
  assert.equal(operationText(running.operation!, false), "Switching to OpenRouter · openai/gpt-5…");
});

test("every row of the command error copy table", () => {
  const idle = view(v2({ saved: SAVED_OR }));
  const cases: Array<[string | null, string, string, string]> = [
    ["not_connected", "", "openrouter", "Connect OpenRouter first."],
    ["sign_in_required", "", "openai_codex", "Sign in to ChatGPT again first."],
    ["credential_rejected", "That's an OpenRouter management key. Use an ordinary API key.", "openrouter", "That's an OpenRouter management key. Use an ordinary API key."],
    ["credential_rejected", "", "openrouter", "OpenRouter didn't accept this key."],
    ["key_allowance_exhausted", "x", "openrouter", "This key has no remaining allowance. Raise its limit at openrouter.ai/keys, or use another key."],
    ["provider_unavailable", "x", "openrouter", "Couldn't reach OpenRouter. Nothing changed. Try again."],
    ["catalog_unavailable", "x", "openai_codex", "ChatGPT's model list is unavailable right now. Your saved model is kept."],
    ["model_unavailable", "x", "openai_codex", "That model isn't available for this ChatGPT account."],
    ["attempt_not_found", "x", "openrouter", "That sign-in is no longer active. Start again."],
    ["operation_in_progress", "x", "openrouter", "Another connection change is still finishing."],
    ["disconnect_in_progress", "x", "openai_codex", "ChatGPT is being removed from this agent. Wait for that to finish, or try the removal again."],
    ["finite_private_unavailable", "x", "openrouter", "Disconnecting would leave this agent without a model: Finite Private isn't fully set up here. Choose another model first."],
    ["facts_unavailable", "x", "openrouter", "The agent couldn't check its setup right now. Try again in a moment."],
    ["activation_not_recorded", "x", "openrouter", "The key was saved, but the agent couldn't record the switch to OpenRouter."],
    ["config_invalid", "Finite Private isn't available on this agent.", "finite_private", "Finite Private isn't available on this agent."],
    ["agent_update_required", "x", "openrouter", "This agent needs an update for this."],
    [null, "Your agent is taking longer than expected. Try again.", "openrouter", "Your agent is taking longer than expected. Try again."],
  ];
  for (const [code, message, route, expected] of cases) {
    assert.equal(copy(commandErrorText(code, message, route as "openrouter", idle)), expected, code ?? "null");
  }
  const busyView = view(v2({ saved: SAVED_OR, operation: operation("disconnect", "openrouter", "running") }));
  assert.equal(
    copy(commandErrorText("operation_in_progress", "x", "finite_private", busyView)),
    "Another connection change is still finishing. Disconnecting OpenRouter… This connection may still be in use until this finishes. The agent's Hermes web dashboard is paused until this finishes."
  );
});

test("the storage-only result is exact, and activation_not_recorded offers Use OpenRouter", () => {
  assert.equal(STORAGE_ONLY_COPY, "Key saved. The agent default is unchanged. OpenRouter conversations may use it.");
  const status = v2({ routes: { openrouter: KEY_SAVED } }, ALL);
  const stored = openRouterCard(status, { card: "openrouter", kind: "stored" });
  assert.match(stored, /data-testid="inference-openrouter-notice">Key saved\. The agent default is unchanged\. OpenRouter conversations may use it\.</u);
  const notRecorded = openRouterCard(status, {
    card: "openrouter", kind: "error", code: "activation_not_recorded", message: "x", route: "openrouter", retryModel: "openai/gpt-5",
  });
  assert.match(text(notRecorded), /The key was saved, but the agent couldn't record the switch to OpenRouter\. Use OpenRouter/u);
  assert.match(notRecorded, /data-testid="inference-openrouter-retry-use"/u);
  assert.doesNotMatch(text(notRecorded), /Nothing changed/u);
  const plain = html(createElement(NoticeLine, {
    notice: { card: "openrouter", kind: "error", code: "credential_rejected", message: "", route: "openrouter" },
    view: view(status),
    testId: "t",
  }));
  assert.doesNotMatch(plain, /retry-use/u);
});

test("every control is disabled before the first status, and the stable test ids are present", () => {
  const markup = html(createElement(ConnectionsPanel, { machineId: "agent-1", googleConfigured: true }));
  const buttons = [...markup.matchAll(/<button\b([^>]*)>(.*?)<\/button>/gu)];
  assert.ok(buttons.length > 3);
  for (const [, attributes, label] of buttons) {
    // The status banner's own "Try again" reloads status, so it is the one control left enabled.
    if (text(label) === "Try again") continue;
    assert.match(attributes, / disabled=""/u, text(label));
  }
  for (const id of [
    "inference-summary", "inference-model-hint", "inference-finite-private", "inference-finite-private-state",
    "inference-finite-private-use", "inference-openrouter", "inference-openrouter-state", "inference-openrouter-open",
  ]) {
    assert.match(markup, new RegExp(`data-testid="${id}"`, "u"), id);
  }
  // before status, nothing says whether ChatGPT is the saved route, so there is no ChatGPT card.
  assert.doesNotMatch(markup, /inference-openrouter-key-input|inference-openrouter-links|inference-codex/u);
});

test("the pasted key: password input with autocomplete off, cleared at submit whether the call succeeds or fails", async () => {
  const form = html(createElement(OpenRouterKeyForm, {
    keyRequired: true, keyPlaceholder: "OpenRouter API key", connect: true, openRouterSaved: false, model: "openai/gpt-5",
    setModel: () => {}, busy: false, onSubmit: async () => {}, onCancel: null,
  }));
  const input = form.match(/<input[^>]*inference-openrouter-key-input[^>]*>/u)?.[0] ?? "";
  assert.match(input, /type="password"/u);
  assert.match(input, /autoComplete="off"|autocomplete="off"/u);
  assert.match(input, /value=""/u);
  assert.match(form, /<form[^>]*method="post"/u);
  // Save only needs a key; the other submit is blocked without one.
  assert.match(form.match(/<button[^>]*inference-openrouter-save-only[^>]*>/u)?.[0] ?? "", / disabled=""/u);

  const events: string[] = [];
  await submitOpenRouterKey("sk-or-v1-fake", () => events.push("cleared"), async (key) => {
    events.push(`sent ${key}`);
  });
  assert.deepEqual(events, ["cleared", "sent sk-or-v1-fake"]);

  events.length = 0;
  await assert.rejects(
    submitOpenRouterKey("sk-or-v1-fake", () => events.push("cleared"), async () => {
      events.push("sending");
      throw new Error("network");
    })
  );
  assert.deepEqual(events, ["cleared", "sending"]);
});

test("the Codex card is read-only in PR1 and says only what the agent reported", () => {
  const savedOnly = text(html(createElement(CodexConnection, { view: view(legacy("openai-codex", "gpt-5.5")) })));
  assert.equal(savedOnly, "ChatGPT Agent default ChatGPT is this agent's default (set in chat).");
  const lines = (status: ReturnType<typeof parseConnectionsStatus>) => codexCardState(view(status)).lines.map(copy).join(" ");
  const codex = (state: string, extra: Inference = {}) =>
    v2({ saved: SAVED_CODEX, routes: { openai_codex: { state, quota_reset_at_ms: null, reported_quota_reset_at_ms: null, login: null } }, ...extra }, ALL);
  assert.equal(lines(codex("not_signed_in")), "Use your personal ChatGPT plan. Work accounts may work if your organization allows it.");
  assert.equal(lines(codex("signed_in")), "Signed in to ChatGPT.");
  assert.equal(lines(codex("sign_in_required")), "ChatGPT can't be used right now. Sign in again if this lasts. Meanwhile Finite Private may answer.");
  assert.equal(lines(codex("sign_in_required", { fallback: fallback("off") })), "ChatGPT can't be used right now. Sign in again if this lasts.");
  assert.equal(lines(codex("quota_limited")), "ChatGPT usage limit reached. Meanwhile Finite Private may answer.");
  assert.equal(lines(codex("unknown")), "Couldn't read the ChatGPT sign-in.");
  assert.equal(lines(codex("signed_in", { operation: operation("disconnect", "openai_codex", "running") })), "Removing ChatGPT from this agent…");
  assert.equal(lines(codex("signed_in", { operation: operation("disconnect", "openai_codex", "failed") })), "Removing ChatGPT didn't finish. It may still be in use.");
  assert.doesNotMatch(html(createElement(CodexConnection, { view: view(codex("signed_in")) })), /<button/u);
});

test("the new components render text only as React text", () => {
  for (const file of ["inference-connections.tsx", "openrouter-connection.tsx", "codex-connection.tsx", "connections-panel.tsx"]) {
    const source = readFileSync(new URL(`../components/${file}`, import.meta.url), "utf8");
    assert.doesNotMatch(source, /dangerouslySetInnerHTML/u, file);
    assert.doesNotMatch(source, /localStorage|sessionStorage|console\./u, file);
  }
});

const CODEX_ROUTE = (state: string) => ({ state, quota_reset_at_ms: null, reported_quota_reset_at_ms: null, login: null });

test("the ChatGPT card shows only for a saved ChatGPT route or an agent with codex.login.v1", () => {
  const cases: Array<[string, ReturnType<typeof parseConnectionsStatus>, string | null, string]> = [
    ["saved, no codex.login.v1", v2({ saved: SAVED_CODEX }), "ChatGPT Agent default ChatGPT is this agent's default (set in chat).", "ChatGPT · gpt-5.3-codex"],
    ["saved, codex.login.v1", v2({ saved: SAVED_CODEX, routes: { openai_codex: CODEX_ROUTE("signed_in") } }, ALL), "ChatGPT Signed in Signed in to ChatGPT.", "ChatGPT · gpt-5.3-codex"],
    ["not saved, no codex.login.v1", v2({ saved: SAVED_OR }), null, "OpenRouter · openai/gpt-5"],
    ["not saved, codex.login.v1", v2({ saved: SAVED_OR, routes: { openai_codex: CODEX_ROUTE("not_signed_in") } }, ALL), "ChatGPT Not connected Use your personal ChatGPT plan. Work accounts may work if your organization allows it.", "OpenRouter · openai/gpt-5"],
  ];
  for (const [name, status, card, summary] of cases) {
    const markup = panel(status);
    assert.match(text(markup), new RegExp(`New conversations use ${summary.replace(/[.]/gu, "\\.")}\\.`, "u"), name);
    const codexCard = markup.match(/<section class="ocean-connection-card" data-testid="inference-codex">.*?<\/section>/u)?.[0];
    assert.equal(codexCard ? text(codexCard) : null, card, name);
    assert.doesNotMatch(text(markup), /Update needed|needs an update to connect ChatGPT/u, name);
  }
});

function lockedButtons(markup: string) {
  const reasonId = markup.match(/<div id="([^"]+)" class="ocean-inference-summary__operation"/u)?.[1];
  assert.ok(reasonId, "the operation line has an id for aria-describedby");
  const button = (id: string) => markup.match(new RegExp(`<button[^>]*data-testid="${id}"[^>]*>`, "u"))?.[0] ?? "";
  return { reasonId, button };
}

test("while an operation runs, every control that would start another change is disabled and names the reason", () => {
  const running = v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED }, operation: operation("select", "openrouter", "running") });
  const markup = panel(running);
  const { reasonId, button } = lockedButtons(markup);
  for (const id of ["inference-finite-private-use", "inference-openrouter-use", "inference-openrouter-replace-key", "inference-openrouter-disconnect"]) {
    assert.match(button(id), / disabled=""/u, id);
    assert.match(button(id), new RegExp(`aria-describedby="${reasonId}"`, "u"), id);
  }
  assert.match(text(markup), /Switching to OpenRouter · openai\/gpt-5…/u);

  // The paste form and its Save buttons, on an agent with and without connect.
  for (const capabilities of [PR1, ALL]) {
    const form = panel(v2({ operation: operation("select", "finite_private", "running", { model: null }) }, capabilities));
    const lockedForm = lockedButtons(form);
    for (const id of capabilities === ALL ? ["inference-openrouter-save-and-use", "inference-openrouter-save-only"] : ["inference-openrouter-save"]) {
      assert.match(lockedForm.button(id), / disabled=""/u, id);
      assert.match(lockedForm.button(id), new RegExp(`aria-describedby="${lockedForm.reasonId}"`, "u"), id);
    }
  }
  // A legacy agent's opener too, though today's agentd never reports an operation.
  const opener = html(createElement(OpenRouterConnection, { view: view(legacy("custom", "glm-5-3-flash")), busy: false, lock: LOCKED, run: noRun, notice: null }));
  assert.match(opener.match(/<button[^>]*inference-openrouter-open[^>]*>/u)?.[0] ?? "", / disabled=""/u);
});

test("controls come back when the operation succeeds or fails", () => {
  for (const state of ["succeeded", "failed"]) {
    const markup = panel(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED }, operation: operation("select", "openrouter", state) }));
    for (const id of ["inference-finite-private-use", "inference-openrouter-use", "inference-openrouter-replace-key", "inference-openrouter-disconnect"]) {
      const tag = markup.match(new RegExp(`<button[^>]*data-testid="${id}"[^>]*>`, "u"))?.[0] ?? "";
      assert.doesNotMatch(tag, / disabled=""|aria-describedby/u, `${id} after ${state}`);
    }
  }
  // Reaching the polling limit must not unlock a running operation.
  const running = view(v2({ operation: operation("select", "openrouter", "running") }));
  assert.equal(controlsLocked(running), true);
  assert.equal(controlsLocked(view(v2({ operation: operation("select", "openrouter", "failed") }))), false);
  assert.equal(controlsLocked(view(legacy("openrouter", "openai/gpt-5"))), false);
  assert.equal(controlsLocked(null), false);
  // "Try again" and "Check again" stay as they were.
  assert.doesNotMatch(operationLine(operation("disconnect", "openrouter", "failed"), { retry: true }).match(/<button[^>]*inference-operation-retry[^>]*>/u)?.[0] ?? "", / disabled=""/u);
  assert.doesNotMatch(operationLine(operation("select", "openrouter", "running"), { pollExpired: true }).match(/<button[^>]*inference-operation-check-again[^>]*>/u)?.[0] ?? "", / disabled=""/u);
});

test("a failed disconnect locks every other change, and Try again stays the way forward", () => {
  const assertLocked = (markup: string, ids: string[], name: string) => {
    const { reasonId, button } = lockedButtons(markup);
    for (const id of ids) {
      assert.match(button(id), / disabled=""/u, `${name}: ${id}`);
      assert.match(button(id), new RegExp(`aria-describedby="${reasonId}"`, "u"), `${name}: ${id}`);
    }
    const retry = button("inference-operation-retry");
    assert.ok(retry, `${name}: Try again is offered`);
    assert.doesNotMatch(retry, / disabled=""/u, `${name}: Try again stays enabled`);
  };
  // OpenRouter's disconnect failed on an agent whose default is ChatGPT: Finite Private's control is locked,
  // and the OpenRouter card shows only its removal-status line.
  const openrouterFailed = panel(v2({
    saved: SAVED_CODEX,
    routes: { openrouter: KEY_SAVED, openai_codex: CODEX_ROUTE("signed_in") },
    operation: operation("disconnect", "openrouter", "failed", { error_code: "verify_failed" }),
  }, ALL));
  assertLocked(openrouterFailed, ["inference-finite-private-use"], "openrouter");
  assert.doesNotMatch(openrouterFailed, /inference-openrouter-use|inference-openrouter-key-input/u);
  // ChatGPT's disconnect failed: Finite Private's and OpenRouter's controls are all locked.
  const codexFailed = panel(v2({
    saved: SAVED_OR,
    routes: { openrouter: KEY_SAVED, openai_codex: CODEX_ROUTE("signed_in") },
    operation: operation("disconnect", "openai_codex", "failed"),
  }, ALL));
  assertLocked(codexFailed, [
    "inference-finite-private-use", "inference-openrouter-use", "inference-openrouter-replace-key", "inference-openrouter-disconnect",
  ], "openai_codex");
  const failed = view(v2({ operation: operation("disconnect", "openrouter", "failed") }));
  assert.equal(controlsLocked(failed), true);
  // A failed select or activate locks nothing.
  for (const kind of ["select", "activate"]) {
    const markup = panel(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED }, operation: operation(kind, "openrouter", "failed") }));
    for (const id of ["inference-finite-private-use", "inference-openrouter-use", "inference-openrouter-disconnect"]) {
      assert.doesNotMatch(markup.match(new RegExp(`<button[^>]*data-testid="${id}"[^>]*>`, "u"))?.[0] ?? "", / disabled=""|aria-describedby/u, `${kind}: ${id}`);
    }
  }
  // Once the disconnect succeeds, everything is enabled again.
  const done = panel(v2({ saved: SAVED_CODEX, routes: { openai_codex: CODEX_ROUTE("signed_in") }, operation: operation("disconnect", "openrouter", "succeeded") }, ALL));
  const doneButton = (id: string) => done.match(new RegExp(`<button[^>]*data-testid="${id}"[^>]*>`, "u"))?.[0] ?? "";
  assert.doesNotMatch(doneButton("inference-finite-private-use"), / disabled=""|aria-describedby/u);
  // Save and use waits only for a key now, not for the operation.
  assert.doesNotMatch(doneButton("inference-openrouter-save-and-use"), /aria-describedby/u);
});

test("the lock follows the last status, and Check again resumes the watch", async () => {
  assert.equal(POLL_LIMIT_MS, 8 * 60_000, "longer than a worst-case disconnect (5 to 7 minutes)");
  const running = view(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED }, operation: operation("disconnect", "openrouter", "running") }));
  const id = running.operation!.id;

  // Before the limit: polling, locked.
  assert.deepStrictEqual(watchState(running, true, null), { delay: 3000, pollKey: id, pollExpired: false, locked: true });
  assert.equal(controlsLocked(running), true);
  // After the limit: no polling, still locked, and Check again offered and enabled.
  assert.deepStrictEqual(watchState(running, true, id), { delay: null, pollKey: id, pollExpired: true, locked: true });
  assert.equal(controlsLocked(running), true, "the polling limit unlocks nothing while the last status shows running");
  const expired = html(createElement(OperationLine, {
    view: running, showDone: false, pollExpired: true, busy: false, onRetry: null, onCheckAgain: () => {},
  }));
  const checkButton = expired.match(/<button[^>]*inference-operation-check-again[^>]*>/u)?.[0] ?? "";
  assert.ok(checkButton, "Check again is offered after the limit");
  assert.doesNotMatch(checkButton, / disabled=""/u);
  // Even while the page's own request is in flight, Check again stays usable.
  const busyLine = html(createElement(OperationLine, {
    view: running, showDone: false, pollExpired: true, busy: true, onRetry: null, onCheckAgain: () => {},
  }));
  assert.doesNotMatch(busyLine.match(/<button[^>]*inference-operation-check-again[^>]*>/u)?.[0] ?? "", / disabled=""/u);

  // Check again reads status once, then clears the expired window.
  let expiredKey: string | null = id;
  let status = running;
  const events: string[] = [];
  await checkAgain(async () => { events.push("read"); }, () => { events.push("resume"); expiredKey = null; });
  assert.deepStrictEqual(events, ["read", "resume"]);
  // Still running: polling resumes, and the page stays locked.
  assert.deepStrictEqual(watchState(status, true, expiredKey), { delay: 3000, pollKey: id, pollExpired: false, locked: true });
  assert.equal(controlsLocked(status), true);
  // A read that fails still clears the window, so the page keeps watching the last status it has.
  expiredKey = id;
  await assert.rejects(checkAgain(async () => { throw new Error("offline"); }, () => { expiredKey = null; }));
  assert.equal(expiredKey, null);

  // Check again returns a status whose operation has ended: nothing to poll, and unlocked.
  for (const state of ["succeeded", "failed"]) {
    status = view(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED }, operation: operation("select", "openrouter", state) }));
    assert.equal(watchState(status, true, id).delay, null, state);
    assert.equal(watchState(status, true, id).locked, false, `a ${state} select unlocks, whatever the window`);
    assert.equal(controlsLocked(status), false, `a ${state} select unlocks`);
  }
  // A failed disconnect stays locked; only Try again moves it on.
  status = view(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED }, operation: operation("disconnect", "openrouter", "failed") }));
  assert.equal(watchState(status, true, id).delay, null);
  assert.equal(watchState(status, true, id).locked, true);
  assert.equal(controlsLocked(status), true);
  // An agent running today's agentd has no operation: never polled for one, never locked.
  const legacyView = view(legacy("openrouter", "openai/gpt-5"));
  assert.deepStrictEqual(watchState(legacyView, true, null), { delay: null, pollKey: null, pollExpired: false, locked: false });
  assert.equal(controlsLocked(legacyView), false);
  // A hidden tab never polls.
  assert.equal(watchState(running, false, null).delay, null);
});

test("while OpenRouter is being removed, its card agrees with the operation line", () => {
  const card = (op: Inference, openrouter: Inference = { state: "no_key", key_source: null, key_hash: null, hermes_key: "none", other_pool_keys: "none" }) =>
    openRouterCard(v2({ routes: { openrouter }, operation: op }));
  for (const [state, line] of [
    ["running", "Removing OpenRouter from this agent…"],
    ["failed", "Removing OpenRouter didn't finish. It may still be in use."],
  ]) {
    for (const openrouter of [undefined, KEY_SAVED]) {
      const markup = card(operation("disconnect", "openrouter", state), openrouter);
      assert.equal(text(markup), `OpenRouter Needs attention ${line}`, `${state} ${openrouter ? "key_saved" : "no_key"}`);
      assert.match(markup, /is-attention/u);
      assert.doesNotMatch(markup, /inference-openrouter-key-input|inference-openrouter-model-input|ocean-connection-card__footer/u);
    }
  }
  const done = card(operation("disconnect", "openrouter", "succeeded"));
  assert.match(text(done), /^OpenRouter Not connected /u);
  assert.match(done, /inference-openrouter-key-input/u);
  // A disconnect of another route leaves this card alone.
  assert.match(text(card(operation("disconnect", "openai_codex", "running"))), /^OpenRouter Not connected /u);
  // A failed disconnect still shows agentd's refusal next to the card if the user acts.
  const refused = openRouterCard(v2({ operation: operation("disconnect", "openrouter", "failed") }), {
    card: "openrouter", kind: "error", code: "operation_in_progress", message: "x", route: "openrouter",
  });
  assert.match(refused, /inference-openrouter-notice/u);
});

test("layout: no empty footer, key-saved controls on their own row, and model IDs that wrap whole", () => {
  const legacyCard = openRouterCard(legacy("custom", "glm-5-3-flash"));
  assert.doesNotMatch(legacyCard, /ocean-connection-card__footer/u, "a legacy agent's card has no empty footer rule");
  const savedCard = openRouterCard(v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED } }));
  assert.doesNotMatch(savedCard, /ocean-connection-card__action/u, "the description keeps the card's full width");
  assert.match(savedCard, /ocean-connection-card__footer.*inference-openrouter-model-input/u);
  const summary = panel(v2());
  assert.match(summary, /<span class="ocean-inference-id"><strong>glm-5-3-flash<\/strong>\.<\/span>/u);
  assert.deepEqual(
    [...summary.matchAll(/<code>(.*?)<\/code>/gu)].map(([, code]) => text(code)),
    ["/model <model>", "--provider", "<finite-private|openrouter>", "--global"]
  );
  assert.match(text(summary), /In chat, \/model <model> --provider <finite-private\|openrouter> switches only that conversation\. Add --global to change this default\./u);
  assert.match(text(panel(v2({ saved: { route: "other", provider: "anthropic", model: "claude" } }))), /New conversations use Custom model · claude, set in Hermes\./u);
});

test("the /model hint names openai-codex only when the ChatGPT card is shown", () => {
  const hint = (markup: string) => text(markup.match(/<p class="ocean-inference-summary__hint"[^>]*>(.*?)<\/p>/u)?.[1] ?? "");
  const withCodex = "In chat, /model <model> --provider <finite-private|openrouter|openai-codex> switches only that conversation. Add --global to change this default.";
  const withoutCodex = "In chat, /model <model> --provider <finite-private|openrouter> switches only that conversation. Add --global to change this default.";
  const cases: Array<[string, ReturnType<typeof parseConnectionsStatus>, string]> = [
    ["ChatGPT saved, no codex.login.v1", v2({ saved: SAVED_CODEX }), withCodex],
    ["ChatGPT saved, codex.login.v1", v2({ saved: SAVED_CODEX, routes: { openai_codex: CODEX_ROUTE("signed_in") } }, ALL), withCodex],
    ["not saved, no codex.login.v1", v2({ saved: SAVED_OR }), withoutCodex],
    ["not saved, codex.login.v1", v2({ saved: SAVED_OR, routes: { openai_codex: CODEX_ROUTE("not_signed_in") } }, ALL), withCodex],
    ["today's agentd, Finite Private saved", legacy("custom", "glm-5-3-flash"), withoutCodex],
    ["today's agentd, OpenRouter saved", legacy("openrouter", "openai/gpt-5"), withoutCodex],
    ["today's agentd, ChatGPT saved", legacy("openai-codex", "gpt-5.5"), withCodex],
  ];
  for (const [name, status, expected] of cases) {
    const markup = panel(status);
    assert.equal(hint(markup), expected, name);
    // The hint and the ChatGPT card always agree.
    assert.equal(hint(markup).includes("openai-codex"), markup.includes('data-testid="inference-codex"'), name);
  }
  assert.equal(hint(panel(null)), withoutCodex, "before status");
});

test("no rendered string claims a saved key or backup works, is ready, valid, or active, or isn't in use", () => {
  // Also render the summary panel across the main states so its strings are scanned.
  for (const status of [
    v2({ saved: SAVED_OR, routes: { openrouter: KEY_SAVED }, operation: operation("activate", "openrouter", "running") }),
    v2({ saved: SAVED_OR, routes: { openrouter: { ...KEY_SAVED, hermes_key: "other_key", other_pool_keys: "present" } } }, ALL),
    v2({ operation: operation("disconnect", "openrouter", "failed") }),
    v2({ saved: { route: "other", provider: "anthropic", model: "claude" }, fallback: fallback("custom") }),
    legacy("openrouter", "openai/gpt-5"),
  ]) {
    panel(status);
  }
  // Copy that uses these words about provider accounts or errors, rather than route readiness.
  const allowed = [
    "The key keeps working in your OpenRouter account until you revoke it there.",
    "That sign-in is no longer active. Start again.",
    "the change isn't valid on this agent",
  ];
  assert.ok(rendered.length > 50);
  for (const entry of rendered) {
    const scrubbed = allowed.reduce((value, sentence) => value.replaceAll(sentence, ""), entry);
    assert.doesNotMatch(scrubbed, /\b(works|working|ready|valid|active)\b|isn't in use|is not in use|not in use/iu, entry);
    // Pre-request fallback may be silent. Do not promise a chat notice until
    // the runtime reports that fallback path as well.
    assert.doesNotMatch(entry, /the chat says so/iu, entry);
  }
});
