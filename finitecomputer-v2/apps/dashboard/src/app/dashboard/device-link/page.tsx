import { LaptopIcon } from "lucide-react";

import { DeviceLinkApproval } from "@/components/device-link-approval";
import { PageHeader } from "@/components/page-header";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { DeviceLinkError, parseDeviceLinkRequest } from "@/lib/device-link";

export const dynamic = "force-dynamic";

type DeviceLinkSearchParams = {
  link_session_id?: string | string[];
  target_device_id?: string | string[];
};

export default async function DeviceLinkPage({
  searchParams,
}: {
  searchParams: Promise<DeviceLinkSearchParams>;
}) {
  const [query, account] = await Promise.all([searchParams, getAccountAuthContext()]);
  const viewerEmail = account.email;
  let input;
  try {
    input = parseDeviceLinkRequest({
      link_session_id: first(query.link_session_id),
      target_device_id: first(query.target_device_id),
    });
  } catch (error) {
    const message =
      error instanceof DeviceLinkError
        ? error.message
        : "This device-link request is invalid.";
    return (
      <div className="space-y-6">
        <PageHeader title="Link Electron" description="Add a local Device to your Finite Chat account." />
        <section className="ocean-utility-card max-w-2xl">
          <div className="ocean-utility-card__header">
            <span className="ocean-utility-card__icon" aria-hidden>
              <LaptopIcon className="size-5" />
            </span>
            <div>
              <h2 className="ocean-utility-card__title">Start again from Electron</h2>
              <p className="text-sm text-muted-foreground">{message}</p>
            </div>
          </div>
        </section>
      </div>
    );
  }

  if (
    !viewerEmail ||
    !account.workosUserId ||
    !account.emailVerified ||
    (account.source !== "workos" && account.source !== "dev")
  ) {
    return (
      <div className="space-y-6">
        <PageHeader title="Link Electron" description="Add a local Device to your Finite Chat account." />
        <section className="ocean-utility-card max-w-2xl">
          <h2 className="ocean-utility-card__title">Sign in again</h2>
          <p className="mt-2 text-sm text-muted-foreground">
            A verified WorkOS account session is required before this Device can be approved.
          </p>
        </section>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Link Electron"
        description="Approve one local Device for your existing Finite Chat account."
      />
      <DeviceLinkApproval input={input} viewerEmail={viewerEmail} />
    </div>
  );
}

function first(value: string | string[] | undefined) {
  return Array.isArray(value) ? value[0] : value;
}
