import { notFound, redirect } from "next/navigation";

import { HostedWebChat } from "@/components/hosted-web-chat";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";

/**
 * Admin preview: the same chat experience as ../chat, rendered over the
 * agent's native Hermes server instead of Finite Chat. The shell mounts the
 * Hermes provider for this path; Core still limits Hermes access to the
 * agent's owner, so an admin can only open agents they own.
 */
export default async function HermesChatPage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) {
    redirect("/dashboard");
  }
  if (!access.viewer?.isAdmin) {
    notFound();
  }
  if (access.machineId !== machineId) {
    redirect(`/dashboard/machines/${encodeURIComponent(access.machineId)}/hermes-chat`);
  }
  return (
    <HostedWebChat
      machineId={access.machineId}
      machineLabel={access.displayName}
      runtimeStatus={access.coreProject.runtime?.runtime_status ?? "unknown"}
    />
  );
}
