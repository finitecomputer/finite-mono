"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { AgentConnectionAction, AgentConnectionsStatus } from "@/lib/hosted-agent-controls";

export function OpenRouterConnection({ status, busy, mutate }: {
  status: AgentConnectionsStatus | null;
  busy: string | null;
  mutate: (label: string, action: AgentConnectionAction, onSuccess?: () => void) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [notice, setNotice] = useState("");
  const [model, setModel] = useState("anthropic/claude-sonnet-4.6");
  const saved = status?.openrouter?.key_saved === true;
  const supported = status?.openrouter !== undefined;
  const validModel = /^[a-zA-Z0-9][a-zA-Z0-9._:/+-]{0,255}$/.test(model);
  const command = validModel ? `/model ${model} --provider openrouter --session` : "";

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(command);
      setNotice("Command copied. Paste it into the session you want to switch.");
    } catch {
      setNotice("Copy the command below and paste it into your chat.");
    }
  }

  return (
    <section className="ocean-connection-card" aria-labelledby="openrouter-title">
      <div className="ocean-connection-card__main">
        <div className="min-w-0">
          <h2 id="openrouter-title" className="ocean-connection-card__name">OpenRouter</h2>
          <p className="mt-1 text-sm font-medium">{!status ? "Checking…" : !supported ? "Runtime update needed" : saved ? "Key saved" : "No key saved"}</p>
          <p className="mt-2 max-w-xl text-sm leading-6 text-muted-foreground">
            Bring your own key for occasional model choices. Your default and current sessions stay unchanged.
          </p>
          {status && !supported ? <p className="mt-2 text-sm text-muted-foreground">Update your agent runtime to connect OpenRouter without changing its default.</p> : null}
        </div>
        <div className="ocean-connection-card__action">
          {editing && supported ? (
            <form className="flex min-w-0 flex-wrap items-center gap-2" onSubmit={(event) => {
              event.preventDefault();
              const form = event.currentTarget;
              const apiKey = String(new FormData(form).get("apiKey") ?? "");
              setNotice("");
              void mutate("openrouter", { action: "openrouter_save", apiKey }, () => {
                form.reset();
                setEditing(false);
                setNotice("Key saved. Your default and session choices are unchanged.");
              });
            }}>
              <Input name="apiKey" type="password" autoComplete="off" aria-label="OpenRouter key" placeholder="OpenRouter API key" required maxLength={16 * 1024} disabled={Boolean(busy)} className="w-60 max-w-full" />
              <Button type="submit" disabled={Boolean(busy)}>{busy === "openrouter" ? "Saving…" : "Save key"}</Button>
              <Button type="button" variant="ghost" disabled={Boolean(busy)} onClick={() => setEditing(false)}>Cancel</Button>
            </form>
          ) : (
            <Button variant="outline" disabled={!supported || Boolean(busy)} onClick={() => { setNotice(""); setEditing(true); }}>
              {saved ? "Replace key" : "Connect OpenRouter"}
            </Button>
          )}
        </div>
      </div>
      {saved ? <div className="ocean-connection-card__footer space-y-3">
        <h3 className="text-sm font-medium">Use a model for one session</h3>
        <p className="text-sm text-muted-foreground">Type <code>/model</code> in chat to see model choices, or copy a command below. OpenRouter receives that session’s requests, including conversation context, and bills your OpenRouter account.</p>
        <div className="flex flex-wrap gap-2">
          <Input aria-label="OpenRouter session model" value={model} onChange={(event) => { setModel(event.target.value); setNotice(""); }} maxLength={256} className="w-72 max-w-full" aria-invalid={!validModel} />
          <Button variant="outline" disabled={!validModel} onClick={() => void copyCommand()}>Copy session command</Button>
        </div>
        {validModel ? <p className="break-all text-sm"><code>{command}</code></p> : <p className="text-sm text-destructive">Enter a model ID, such as anthropic/claude-sonnet-4.6, without spaces or command flags.</p>}
        <p className="text-sm text-muted-foreground"><code>/model private</code> returns this conversation to Finite Private. <code>/new</code> starts a session using your default. Earlier requests to OpenRouter cannot be recalled.</p>
        <p className="text-xs text-muted-foreground">Saving a key does not verify it with OpenRouter. It is checked when you use a model.</p>
      </div> : null}
      {notice ? <p role="status" className="ocean-connection-card__footer text-sm">{notice}</p> : null}
    </section>
  );
}
