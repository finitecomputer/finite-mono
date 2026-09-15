import { notFound } from "next/navigation";

import { BrainOverviewPrototype } from "./prototype";

// FIN-89: a throwaway visual study, reachable only in the local design fixture.
export default async function BrainPrototypePage({
  params,
}: {
  params: Promise<{ machineId: string }>;
}) {
  const { machineId } = await params;
  if (
    process.env.NODE_ENV !== "development"
    || process.env.FC_DASHBOARD_ALLOW_DEV_ACCOUNT_AUTH !== "1"
    || machineId !== "runtime_web_design"
  ) {
    notFound();
  }
  return <BrainOverviewPrototype />;
}
