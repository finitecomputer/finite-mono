import { redirect } from "next/navigation";

import { HostedWebChat } from "@/components/hosted-web-chat";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";
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
  const [access, viewer] = await Promise.all([
    loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" }),
    loadOptionalViewerContext(),
  ]);
  if (!access) {
    redirect("/dashboard");
  }
  return (
    <HostedWebChat
      connectionsHref={productHref(
        process.env.FC_ACCOUNT_CONNECTIONS_URL,
        `/dashboard/machines/${encodeURIComponent(access.machineId)}/connections`,
        access.machineId
      )}
      initialDraft={initialDraft(query.prompt)}
      machineId={access.machineId}
      machineLabel={access.displayName}
      showSkills={viewer.isAdmin}
      viewerEmail={viewer.email}
    />
  );
}

function productHref(
  value: string | undefined,
  fallback: string | null,
  machineId: string
) {
  const candidate = value?.trim().replaceAll("{machineId}", encodeURIComponent(machineId));
  if (!candidate) return fallback;
  if (candidate.startsWith("/")) return candidate;
  try {
    const url = new URL(candidate);
    return url.protocol === "https:" || url.protocol === "http:" ? url.toString() : null;
  } catch {
    return null;
  }
}

function initialDraft(value: string | string[] | undefined) {
  const prompt = Array.isArray(value) ? value[0] : value;
  return prompt?.trim().slice(0, 4_000) ?? "";
}
