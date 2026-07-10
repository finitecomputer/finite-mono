import Link from "next/link";
import { redirect } from "next/navigation";
import {
  BriefcaseBusinessIcon,
  CpuIcon,
  MessageSquareIcon,
  SendIcon,
  ShieldCheckIcon,
} from "lucide-react";

import { ConnectionCard } from "@/components/connection-card";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export default async function MachineConnectionsPage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) redirect("/dashboard");

  return (
    <div className="space-y-6">
      <PageHeader
        eyebrow={access.machineId}
        title="Connections"
        description={`Connect ${access.displayName} without turning the Runner into a product control plane.`}
      />

      <section className="ocean-utility-card">
        <div className="ocean-utility-card__header">
          <span className="ocean-utility-card__icon" aria-hidden>
            <ShieldCheckIcon className="size-5" />
          </span>
          <div>
            <h2 className="ocean-utility-card__title">Safe first step</h2>
            <p className="ocean-utility-card__description">
              These actions prepare a request in the canonical encrypted chat. Nothing changes until
              you review and send it. A small agent-local executor will later own validated apply,
              health checks, and rollback for this same four-connection surface.
            </p>
          </div>
        </div>
      </section>

      <div className="space-y-4">
        <ConnectionCard
          name="Inference"
          state="agent-guided"
          account="Finite Private or OpenRouter"
          description="Inspect the active Hermes model, keep Finite Private as the default, or prepare an OpenRouter switch. Secrets are never placed in the link."
          icon={<CpuIcon className="size-5" />}
          footer={<ConnectionScope items={["Finite Private", "OpenRouter", "Model selection", "Rollback plan"]} />}
        >
          <Button asChild>
            <Link href={chatPromptHref(access.machineId, INFERENCE_PROMPT)}>
              <MessageSquareIcon />
              Configure in chat
            </Link>
          </Button>
        </ConnectionCard>

        <ConnectionCard
          name="Telegram"
          state="agent-guided"
          description="Check Hermes’ Telegram support, prepare pairing, and establish the default chat without editing runtime environment files from the dashboard."
          icon={<SendIcon className="size-5" />}
          footer={<ConnectionScope items={["Bot connection", "Pairing", "Default chat", "Disconnect"]} />}
        >
          <Button asChild variant="outline">
            <Link href={chatPromptHref(access.machineId, TELEGRAM_PROMPT)}>
              <MessageSquareIcon />
              Set up Telegram
            </Link>
          </Button>
        </ConnectionCard>

        <ConnectionCard
          name="Google Workspace"
          state="agent-guided"
          description="Prepare the agent-owned OAuth flow for Gmail, Calendar, Drive, Docs, Sheets, and Slides using the canonical bounded scope contract."
          icon={<BriefcaseBusinessIcon className="size-5" />}
          footer={<ConnectionScope items={["Google OAuth", "Bounded scopes", "Reconnect", "Reset"]} />}
        >
          <Button asChild variant="outline">
            <Link href={chatPromptHref(access.machineId, GOOGLE_PROMPT)}>
              <MessageSquareIcon />
              Connect Google
            </Link>
          </Button>
        </ConnectionCard>
      </div>
    </div>
  );
}

function ConnectionScope({ items }: { items: string[] }) {
  return (
    <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
      {items.map((item) => (
        <span key={item} className="rounded-full border border-border/70 px-2 py-1">
          {item}
        </span>
      ))}
    </div>
  );
}

function chatPromptHref(machineId: string, prompt: string) {
  return `/dashboard/machines/${encodeURIComponent(machineId)}/chat?prompt=${encodeURIComponent(prompt)}`;
}

const INFERENCE_PROMPT =
  "Help me inspect and configure this agent's inference connection. Only Finite Private and OpenRouter are in scope. Explain the current state and proposed change before applying anything. Do not ask me to paste a secret until a safe agent-local apply path is available.";
const TELEGRAM_PROMPT =
  "Help me connect Telegram to this agent using Hermes' supported configuration flow. Inspect current status first, explain pairing and rollback, and do not edit a remote .env file.";
const GOOGLE_PROMPT =
  "Help me connect Google Workspace to this agent using the installed google-workspace-finite skill and its bounded scope contract. Inspect prerequisites first and explain exactly what the OAuth flow will grant before starting it.";
