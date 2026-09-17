import Link from "next/link";
import { redirect } from "next/navigation";

import { AgentSkillsBrowser } from "@/components/agent-skills-browser";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import headingStyles from "@/styles/agent-page-heading.module.css";

export const dynamic = "force-dynamic";

export default async function SkillsDashboardPage({ searchParams }: {
  searchParams: Promise<{ machine?: string | string[] }>;
}) {
  const { machine } = await searchParams;
  if (typeof machine !== "string" || !machine.trim()) {
    return <div className={`${headingStyles.page} space-y-4`}>
      <h1 className={headingStyles.title}>Skills</h1>
      <p className={headingStyles.subtitle}>Choose an agent to see its skills.</p>
      <Link href="/dashboard" className="underline">Choose an agent</Link>
    </div>;
  }
  const [access, account] = await Promise.all([
    loadDashboardMachineAccess(machine), getAccountAuthContext(),
  ]);
  if (!access) redirect("/dashboard");
  if (access.machineId !== machine) redirect(`/dashboard/skills?machine=${encodeURIComponent(access.machineId)}`);
  const scope = JSON.stringify([account.workosUserId ?? account.email, account.organizationId ?? null, access.machineId]);

  return <div className={`ocean-page-stack ${headingStyles.page}`}>
    <AgentSkillsBrowser key={scope} runtimeId={access.machineId} agentName={access.displayName} />
  </div>;
}
