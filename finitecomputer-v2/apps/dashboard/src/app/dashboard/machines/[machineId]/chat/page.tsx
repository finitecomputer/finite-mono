import { redirect } from "next/navigation";

import { HostedWebChat } from "@/components/hosted-web-chat";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

export default async function HostedWebChatPage({
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
      `/dashboard/machines/${encodeURIComponent(access.machineId)}/chat`,
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
    />
  );
}

function initialDraft(value: string | string[] | undefined) {
  const prompt = Array.isArray(value) ? value[0] : value;
  return prompt?.trim().slice(0, 4_000) ?? "";
}
