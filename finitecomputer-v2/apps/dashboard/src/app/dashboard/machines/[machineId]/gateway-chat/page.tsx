import { redirect } from "next/navigation";

import { HostedWebChat } from "@/components/hosted-web-chat";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

/**
 * Spike: the same chat experience as ../chat, with the provider swapped at
 * the shell level — the unmodified HostedWebChat tree renders over the
 * hermes tui_gateway WebSocket instead of the finitechat hosted device.
 */
export default async function GatewayChatPage({
  params,
  searchParams,
}: {
  params: Promise<{ machineId: string }>;
  searchParams: Promise<{ prompt?: string | string[] }>;
}) {
  const { machineId } = await params;
  const query = await searchParams;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) {
    redirect("/dashboard");
  }
  if (access.machineId !== machineId) {
    const destination = new URL(
      `/dashboard/machines/${encodeURIComponent(access.machineId)}/gateway-chat`,
      "https://finite.invalid"
    );
    const prompt = Array.isArray(query.prompt) ? query.prompt[0] : query.prompt;
    if (prompt) destination.searchParams.set("prompt", prompt);
    redirect(`${destination.pathname}${destination.search}`);
  }
  return (
    <HostedWebChat
      initialDraft={initialDraft(query.prompt)}
      machineId={access.machineId}
      machineLabel={access.displayName}
      runtimeStatus={access.coreProject.runtime?.runtime_status ?? "unknown"}
    />
  );
}

function initialDraft(value: string | string[] | undefined) {
  const prompt = Array.isArray(value) ? value[0] : value;
  return prompt?.trim().slice(0, 4_000) ?? "";
}
