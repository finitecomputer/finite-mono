"use client";

import { useState, type FormEvent, type ReactNode } from "react";
import { ExternalLinkIcon, KeyRoundIcon, UnplugIcon } from "lucide-react";

import { ConnectionCard } from "@/components/connection-card";
import {
  NoticeLine,
  removalText,
  type InferenceLock,
  type InferenceNotice,
  type InferenceRun,
} from "@/components/inference-connections";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { AgentConnectionAction } from "@/lib/hosted-agent-controls";
import type { InferenceView } from "@/lib/inference-status";

const OPENROUTER_DEFAULT_MODEL = "anthropic/claude-sonnet-4.6";
const KEY_HASH = /^[0-9a-f]{64}$/u;

type KeySubmit = "use" | "store" | "legacy";
/** A disconnect this page started: its operation (null when nothing needed removing) and the key it removed. */
type Disconnect = { operationId: string | null; keyHash: string | null };

export function OpenRouterConnection({
  view,
  busy,
  lock,
  run,
  notice,
}: {
  view: InferenceView | null;
  busy: boolean;
  lock: InferenceLock;
  run: InferenceRun;
  notice: InferenceNotice | null;
}) {
  const [modelDraft, setModelDraft] = useState<string | null>(null);
  const [keyFormOpen, setKeyFormOpen] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const [disconnect, setDisconnect] = useState<Disconnect | null>(null);

  const saved = view?.saved.route === "openrouter";
  const model = modelDraft ?? (saved && view?.saved.model ? view.saved.model : OPENROUTER_DEFAULT_MODEL);
  const state = view?.v2 ? view.openrouter.state : null;
  const canConnect = Boolean(view?.capabilities.has("openrouter.connect.v1"));
  const canDisconnect = Boolean(view?.capabilities.has("inference.disconnect.v1"));
  // with no key known, the card offers the key form; `unknown` is offered what `no_key` is.
  const noKnownKey = state === "no_key" || state === "unknown";
  const keyFormShown = Boolean(view) && (keyFormOpen || noKnownKey);
  const removed =
    disconnect &&
    (disconnect.operationId === null ||
      (view?.operation?.id === disconnect.operationId && view.operation.state === "succeeded"));
  const card = openRouterCardState(view);
  // while another change runs, nothing here may start one; the operation line says why.
  const disabled = busy || lock.locked;
  const describedBy = lock.locked ? lock.reasonId : undefined;
  // while OpenRouter is being removed, the card says only that, in agreement with the operation line.
  const removing = view ? removalText(view, "openrouter") : null;

  async function selectModel(nextModel: string) {
    if (!view) return;
    await run("openrouter", openRouterUseAction(view, nextModel), { route: "openrouter" });
  }

  async function submitKey(action: AgentConnectionAction, submit: KeySubmit) {
    const outcome = await run("openrouter", action, {
      route: "openrouter",
      retryModel: submit === "store" ? undefined : model,
    });
    if (outcome.ok) {
      setKeyFormOpen(false);
      setDisconnect(null);
    }
  }

  async function confirmRemoval() {
    if (!view) return;
    setConfirmDisconnect(false);
    const keyHash = view.openrouter.keyHash;
    const outcome = await run("openrouter", { action: "inference_disconnect", route: "openrouter" }, { route: "openrouter" });
    if (!outcome.ok) return;
    const result = outcome.result;
    setDisconnect({ operationId: result && "accepted" in result ? result.operation_id : null, keyHash });
  }

  const footer: ReactNode[] = [];
  if (view && !removing) {
    if (keyFormShown) {
      footer.push(
        <OpenRouterKeyForm
          key="key-form"
          keyRequired={Boolean(view.v2)}
          keyPlaceholder={view.v2 ? "OpenRouter API key" : saved ? "New key (optional)" : "Your key (optional)"}
          connect={canConnect}
          openRouterSaved={saved}
          model={model}
          setModel={setModelDraft}
          busy={disabled}
          describedBy={describedBy}
          onSubmit={submitKey}
          onCancel={noKnownKey ? null : () => setKeyFormOpen(false)}
        />
      );
    } else if (view.v2) {
      // With a key saved, the controls get their own row so the description keeps the card's width.
      footer.push(
        <div key="actions" className="flex min-w-0 flex-wrap items-center gap-2">
          <Input
            value={model}
            onChange={(event) => setModelDraft(event.target.value)}
            aria-label="OpenRouter model"
            autoComplete="off"
            className="w-60"
            data-testid="inference-openrouter-model-input"
          />
          <Button
            disabled={disabled || !model.trim()}
            aria-describedby={describedBy}
            data-testid="inference-openrouter-use"
            onClick={() => void selectModel(model)}
          >
            {saved ? "Save model" : "Use OpenRouter"}
          </Button>
          <Button
            variant="outline"
            disabled={disabled}
            aria-describedby={describedBy}
            data-testid="inference-openrouter-replace-key"
            onClick={() => setKeyFormOpen(true)}
          >
            Replace key
          </Button>
          {canDisconnect ? (
            <Button
              variant="outline"
              disabled={disabled}
              aria-describedby={describedBy}
              data-testid="inference-openrouter-disconnect"
              onClick={() => setConfirmDisconnect(true)}
            >
              <UnplugIcon />
              Disconnect
            </Button>
          ) : null}
        </div>
      );
    }
    for (const line of openRouterDetailLines(view)) {
      footer.push(
        <p key={line.testId} className="text-sm text-muted-foreground" data-testid={line.testId}>
          {line.text}
        </p>
      );
    }
  }
  if (view && notice) {
    footer.push(
      <NoticeLine
        key="notice"
        notice={notice}
        view={view}
        testId="inference-openrouter-notice"
        busy={disabled}
        describedBy={describedBy}
        onRetryUse={(retryModel) => void selectModel(retryModel)}
      />
    );
  }
  if (view && !removing) {
    if (removed && disconnect) {
      footer.push(<OpenRouterRemoved key="removed" view={view} keyHash={disconnect.keyHash} />);
    }
    if (!canDisconnect && (saved || state === "key_saved")) {
      footer.push(
        <p key="update-needed" className="text-sm text-muted-foreground" data-testid="inference-openrouter-disconnect-update-needed">
          This agent needs an update to disconnect here.
        </p>
      );
    }
    if (view.openrouter.state === "key_saved" || saved) {
      footer.push(<OpenRouterLinks key="links" keyHash={view.openrouter.keyHash} />);
    }
  }

  return (
    <>
      <ConnectionCard
        name="OpenRouter"
        state={card.state}
        statusLabel={card.label}
        note={card.note}
        account={view && !removing ? openRouterAccountLine(view) : null}
        description={removing ? <span data-testid="inference-openrouter-removing">{removing}</span> : openRouterDescription(view)}
        icon={<KeyRoundIcon className="size-5" />}
        testId="inference-openrouter"
        footer={footer.length ? footer : null}
      >
        {!view || (!view.v2 && !keyFormOpen) ? (
          <Button
            variant="outline"
            disabled={!view || lock.locked}
            aria-describedby={describedBy}
            data-testid="inference-openrouter-open"
            onClick={() => setKeyFormOpen(true)}
          >
            Use OpenRouter
          </Button>
        ) : null}
      </ConnectionCard>
      <Dialog open={confirmDisconnect} onOpenChange={setConfirmDisconnect}>
        <DialogContent data-testid="inference-openrouter-disconnect-dialog">
          <DialogHeader>
            <DialogTitle>{OPENROUTER_DISCONNECT_TITLE}</DialogTitle>
            <DialogDescription>{view ? openRouterDisconnectCopy(view) : null}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" data-testid="inference-openrouter-disconnect-cancel" onClick={() => setConfirmDisconnect(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={disabled}
              aria-describedby={describedBy}
              data-testid="inference-openrouter-disconnect-confirm"
              onClick={() => void confirmRemoval()}
            >
              Disconnect
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

/**
 * The pasted key is held only in this form's state. It is cleared the moment the form is submitted,
 * before the request settles, so it survives neither success nor failure.
 */
function OpenRouterKeyForm({
  keyRequired,
  keyPlaceholder,
  connect,
  openRouterSaved,
  model,
  setModel,
  busy,
  describedBy,
  onSubmit,
  onCancel,
}: {
  keyRequired: boolean;
  keyPlaceholder: string;
  connect: boolean;
  /** OpenRouter is the saved default already, so the button text alone says what Save does. */
  openRouterSaved: boolean;
  model: string;
  setModel: (model: string) => void;
  /** True while a request is in flight or another change is running. */
  busy: boolean;
  /** The operation line, for a screen reader that reaches a disabled control. */
  describedBy?: string;
  onSubmit: (action: AgentConnectionAction, submit: KeySubmit) => Promise<void>;
  onCancel: (() => void) | null;
}) {
  const [apiKey, setApiKey] = useState("");
  const submit = (kind: KeySubmit) =>
    submitOpenRouterKey(apiKey, kind, model, () => setApiKey(""), (action) => onSubmit(action, kind));
  const missing = (keyRequired && !apiKey.trim()) || !model.trim();
  return (
    <form
      method="post"
      className="flex min-w-0 flex-wrap items-center gap-2"
      onSubmit={(event: FormEvent<HTMLFormElement>) => {
        event.preventDefault();
        if (!missing && !busy) void submit(connect ? "use" : "legacy");
      }}
    >
      <Input
        name="apiKey"
        type="password"
        autoComplete="off"
        value={apiKey}
        onChange={(event) => setApiKey(event.target.value)}
        placeholder={keyPlaceholder}
        aria-label="OpenRouter key"
        className="w-52"
        data-testid="inference-openrouter-key-input"
      />
      <Input
        name="model"
        value={model}
        onChange={(event) => setModel(event.target.value)}
        aria-label="OpenRouter model"
        autoComplete="off"
        className="w-60"
        data-testid="inference-openrouter-key-model-input"
      />
      {connect ? (
        <>
          <Button type="submit" disabled={busy || missing} aria-describedby={describedBy} data-testid="inference-openrouter-save-and-use">
            Save and use
          </Button>
          <Button
            type="button"
            variant="outline"
            disabled={busy || !apiKey.trim()}
            aria-describedby={describedBy}
            data-testid="inference-openrouter-save-only"
            onClick={() => void submit("store")}
          >
            Save only
          </Button>
        </>
      ) : (
        <Button type="submit" disabled={busy || missing} aria-describedby={describedBy} data-testid="inference-openrouter-save">
          Save and use OpenRouter
        </Button>
      )}
      {onCancel ? (
        <Button
          type="button"
          variant="ghost"
          data-testid="inference-openrouter-cancel-key"
          onClick={() => {
            setApiKey("");
            onCancel();
          }}
        >
          Cancel
        </Button>
      ) : null}
      {connect || openRouterSaved ? null : (
        // v1 apply always saves OpenRouter as the default, for a first key and for "Replace key" alike.
        <p className="basis-full text-sm text-muted-foreground" data-testid="inference-openrouter-key-default-line">
          This also makes OpenRouter this agent&apos;s default.
        </p>
      )}
    </form>
  );
}

export async function submitOpenRouterKey(
  apiKey: string,
  submit: KeySubmit,
  model: string,
  clearKey: () => void,
  send: (action: AgentConnectionAction) => Promise<void>
) {
  clearKey();
  await send(openRouterKeyAction(submit, apiKey, model));
}

/** client orchestration: one call per intent, the legacy action for agents without the capability. */
function openRouterKeyAction(submit: KeySubmit, apiKey: string, model: string): AgentConnectionAction {
  const key = apiKey.trim();
  if (submit === "legacy") {
    return { action: "inference", profile: "openrouter", ...(key ? { apiKey: key } : {}), model: model.trim() };
  }
  return submit === "use"
    ? { action: "openrouter_connect_key", apiKey: key, activate: { model: model.trim() } }
    : { action: "openrouter_connect_key", apiKey: key };
}

function openRouterUseAction(view: InferenceView, model: string): AgentConnectionAction {
  return view.capabilities.has("inference.select.v1")
    ? { action: "inference_select", route: "openrouter", model: model.trim() }
    : { action: "inference", profile: "openrouter", model: model.trim() };
}

function openRouterCardState(view: InferenceView | null): {
  state: "connected" | "disconnected" | "attention" | "unavailable";
  label?: string;
  note?: string;
} {
  if (!view) return { state: "unavailable" };
  if (removalText(view, "openrouter")) return { state: "attention" };
  if (!view.v2) {
    return view.saved.route === "openrouter"
      ? { state: "connected", label: "Agent default" }
      : { state: "disconnected", label: "Not the agent default" };
  }
  if (view.openrouter.state === "key_saved") return { state: "connected", label: "Key saved" };
  if (view.openrouter.state === "no_key") return { state: "disconnected" };
  // no saved key, and the agent couldn't check whether Hermes has one.
  return { state: "unavailable", label: "Not confirmed", note: "The agent couldn't confirm its OpenRouter setup right now." };
}

function openRouterDescription(view: InferenceView | null) {
  return view?.fallback.state === "configured"
    ? "Use your own OpenRouter account. Finite Private is configured as a backup."
    : "Use your own OpenRouter account.";
}

function openRouterAccountLine(view: InferenceView) {
  if (!view.v2 || view.openrouter.state !== "key_saved") return null;
  switch (view.openrouter.keySource) {
    case "agent":
      return "Key saved in this agent";
    case "legacy_config":
      return "Key saved in an older format. Select Use OpenRouter to finish setting it up.";
    case "environment":
      return "Key provided by this agent's environment";
    default:
      return null;
  }
}

function openRouterDetailLines(view: InferenceView) {
  const lines: Array<{ testId: string; text: string }> = [];
  if (!view.v2 || view.openrouter.state !== "key_saved") return lines;
  if (view.openrouter.hermesKey === "other_key") {
    lines.push({
      testId: "inference-openrouter-hermes-key-line",
      text: "Hermes is set up to use a different OpenRouter key than the one saved here.",
    });
  }
  if (view.openrouter.otherPoolKeys === "present") {
    lines.push({
      testId: "inference-openrouter-pool-line",
      text: "Hermes also has other OpenRouter keys, added in chat or the terminal, and may use them.",
    });
  }
  return lines;
}

const OPENROUTER_DISCONNECT_TITLE = "Remove OpenRouter from this agent?";

function openRouterDisconnectCopy(view: InferenceView) {
  return [
    "Finite deletes the key saved in this agent and the agent's other copies, and restarts the agent's model service.",
    "Programs the agent started earlier may keep a copy until they stop.",
    "The key keeps working in your OpenRouter account until you revoke it there.",
    ...(view.saved.route === "openrouter" ? ["This agent will switch to Finite Private."] : []),
    "Conversations that picked OpenRouter with /model will use the agent default.",
  ].join(" ");
}

function openRouterKeyUrl(keyHash: string | null) {
  return keyHash && KEY_HASH.test(keyHash)
    ? `https://openrouter.ai/keys/${keyHash}`
    : "https://openrouter.ai/settings/keys";
}

function openRouterLinks(keyHash: string | null) {
  const hash = keyHash && KEY_HASH.test(keyHash) ? keyHash : null;
  return [
    { testId: "inference-openrouter-link-add-funds", label: "Add funds", href: "https://openrouter.ai/settings/credits" },
    {
      testId: "inference-openrouter-link-account-credits",
      label: "Account credits",
      href: "https://openrouter.ai/settings/credits",
      caption: "Your account balance is shown on OpenRouter.",
    },
    { testId: "inference-openrouter-link-manage-key", label: "Manage this key", href: openRouterKeyUrl(hash) },
    {
      testId: "inference-openrouter-link-key-activity",
      label: "Key activity",
      href: hash ? `https://openrouter.ai/logs?api_key_hash=${hash}` : "https://openrouter.ai/activity",
    },
  ];
}

function OpenRouterLinks({ keyHash }: { keyHash: string | null }) {
  return (
    <div className="grid gap-2" data-testid="inference-openrouter-links">
      <ul className="ocean-inference-links">
        {openRouterLinks(keyHash).map((link) => (
          <li key={link.testId}>
            <a href={link.href} target="_blank" rel="noopener noreferrer" data-testid={link.testId}>
              {link.label}
              <ExternalLinkIcon className="size-3.5" aria-hidden />
            </a>
            {link.caption ? <span className="text-muted-foreground"> {link.caption}</span> : null}
          </li>
        ))}
      </ul>
      <p className="text-xs text-muted-foreground">
        Key links only work when you&apos;re signed in to the OpenRouter account that owns the key.
      </p>
    </div>
  );
}

export function OpenRouterRemoved({ view, keyHash }: { view: InferenceView; keyHash: string | null }) {
  // A new stored key supersedes this page's earlier removal. Environment
  // credentials can intentionally remain after disconnect and are explained below.
  if (view.openrouter.state === "key_saved" && view.openrouter.keySource !== "environment") return null;
  return (
    <div className="grid gap-1 text-sm" role="status" aria-live="polite" data-testid="inference-openrouter-removed">
      <p>
        OpenRouter was removed from this agent.{" "}
        <a
          href={openRouterKeyUrl(keyHash)}
          target="_blank"
          rel="noopener noreferrer"
          className="font-medium underline underline-offset-4"
          data-testid="inference-openrouter-revoke-link"
        >
          Revoke the key in OpenRouter
        </a>
      </p>
      {view.openrouter.keySource === "environment" ? (
        <p className="text-muted-foreground" data-testid="inference-openrouter-environment-key">
          This agent still gets an OpenRouter key from its environment, so OpenRouter can still be used.
        </p>
      ) : null}
    </div>
  );
}
