import Link from "next/link";
import { notFound } from "next/navigation";
import { ActivityIcon, ArrowLeftIcon } from "lucide-react";

import { adminOpsUpgradeRuntimeAction } from "@/app/actions";
import { FormActionButton } from "@/components/form-action-button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { canAccessAdminOps } from "@/lib/admin-ops";
import { loadCoreAdminRuntimes } from "@/lib/core-client";
import { loadOptionalViewerContext } from "@/lib/dashboard-auth";

type RuntimeUpgradePageProps = {
  searchParams: Promise<{ projectId?: string | string[] }>;
};

export default async function RuntimeUpgradePage({
  searchParams,
}: RuntimeUpgradePageProps) {
  const viewer = await loadOptionalViewerContext();
  if (!canAccessAdminOps(viewer)) {
    notFound();
  }

  const query = await searchParams;
  const projectId = Array.isArray(query.projectId)
    ? query.projectId[0]
    : query.projectId;
  if (!projectId) {
    notFound();
  }

  const result = await loadCoreAdminRuntimes();
  const runtime = result.runtimes?.find(
    (candidate) => candidate.project_id === projectId
  );
  if (!result.configured || result.error || !runtime) {
    notFound();
  }

  return (
    <div className="ocean-page-stack">
      <Button asChild variant="ghost" size="sm" className="w-fit">
        <Link href="/dashboard/admin">
          <ArrowLeftIcon />
          Admin Ops
        </Link>
      </Button>

      <section className="ocean-utility-card max-w-2xl">
        <div className="ocean-utility-card__header">
          <span className="ocean-utility-card__icon" aria-hidden>
            <ActivityIcon className="size-5" />
          </span>
          <div>
            <h1 className="ocean-utility-card__title">Upgrade hosted runtime</h1>
            <p className="text-sm text-muted-foreground">
              Operator control for {runtime.project_display_name}. Core will
              apply the exact approved artifact ID to this runtime on its
              existing volume.
            </p>
          </div>
        </div>

        {!runtime.supports_runtime_control ? (
          <div className="ocean-empty-state">
            Runtime control is not available for this project.
          </div>
        ) : (
          <form action={adminOpsUpgradeRuntimeAction} className="grid gap-3">
            <input type="hidden" name="projectId" value={runtime.project_id} />
            <label
              className="grid gap-1.5 text-sm font-medium text-foreground"
              htmlFor="targetRuntimeArtifactId"
            >
              Exact target runtime artifact ID
            </label>
            <Input
              id="targetRuntimeArtifactId"
              name="targetRuntimeArtifactId"
              className="font-mono text-xs"
              placeholder="Paste an approved artifact ID"
              required
              autoComplete="off"
              spellCheck={false}
            />
            <p className="text-xs text-muted-foreground">
              No candidate is selected automatically. Verify the complete ID
              before continuing.
            </p>
            <FormActionButton className="w-fit" pendingLabel="Upgrading...">
              <ActivityIcon />
              Upgrade runtime
            </FormActionButton>
          </form>
        )}
      </section>
    </div>
  );
}
