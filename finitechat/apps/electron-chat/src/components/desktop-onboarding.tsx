import { Loader2Icon, ShieldCheckIcon } from "lucide-react";

import { FiniteBrand } from "./finite-brand";

export type DesktopIdentityStatus = {
  secureStorageAvailable: boolean;
  hasStoredAccountSecret: boolean;
  linking: boolean;
};

export type DesktopOnboardingStatus = {
  completed: boolean;
};

export type DesktopDeviceLinkReady = {
  link_session_id: string;
  target_device_id: string;
  approval_url: string;
};

export type DesktopDeviceLinkStatus =
  | { status: "idle" }
  | { status: "waiting"; ready: DesktopDeviceLinkReady }
  | { status: "linked" }
  | { status: "failed"; message: string }
  | { status: "cancelled" };

export function DesktopOnboarding({
  busy,
  deviceLinkStatus,
  error,
  identityStatus,
  onBeginLink,
  onCancelLink,
  onOpenApproval,
  onUseLinkedAccount,
}: {
  busy: boolean;
  deviceLinkStatus: DesktopDeviceLinkStatus;
  error: string | null;
  identityStatus: DesktopIdentityStatus | null;
  onBeginLink: () => void;
  onCancelLink: () => void;
  onOpenApproval: (ready: DesktopDeviceLinkReady) => void;
  onUseLinkedAccount: () => void;
}) {
  const waitingReady = deviceLinkStatus.status === "waiting" ? deviceLinkStatus.ready : null;
  const waiting = waitingReady !== null;
  const linked = deviceLinkStatus.status === "linked";
  const hasLinkedAccount = identityStatus?.hasStoredAccountSecret === true || linked;
  const primaryAction = hasLinkedAccount
    ? onUseLinkedAccount
    : waitingReady
      ? () => onOpenApproval(waitingReady)
      : onBeginLink;
  const primaryTitle = hasLinkedAccount
    ? "Continue with linked account"
    : waiting
      ? "Open approval in browser"
      : "Link with Finite Computer";
  const primaryDetail = hasLinkedAccount
    ? linked
      ? "Finishing local setup and opening your existing encrypted chats"
      : "Account key stored in this computer's secure storage"
    : waiting
      ? "Approve this Device from your signed-in Finite Computer account"
      : "Use the same account and conversations as the web app";

  return (
    <div
      className="desktop-onboarding"
      role="dialog"
      aria-modal="true"
      aria-labelledby="desktop-onboarding-title"
    >
      <section className="desktop-onboarding__panel">
        <div className="desktop-onboarding__brand">
          <FiniteBrand />
          <span>Desktop</span>
        </div>
        <div className="desktop-onboarding__copy">
          <h1 id="desktop-onboarding-title">Finite Chat</h1>
          <p>
            Link this computer to your Finite Computer account. It becomes its own revocable
            Device while the account key remains in local secure storage.
          </p>
        </div>

        <button
          type="button"
          className="desktop-onboarding__choice"
          onClick={primaryAction}
          disabled={
            busy || (!hasLinkedAccount && identityStatus?.secureStorageAvailable === false)
          }
        >
          {busy ? <Loader2Icon className="desktop-spin" aria-hidden /> : <ShieldCheckIcon aria-hidden />}
          <span>
            <strong>{primaryTitle}</strong>
            <small>{primaryDetail}</small>
          </span>
        </button>

        {waiting ? (
          <button
            type="button"
            className="desktop-onboarding__cancel"
            onClick={onCancelLink}
            disabled={busy}
          >
            Cancel this link request
          </button>
        ) : null}

        {identityStatus?.secureStorageAvailable === false ? (
          <div className="desktop-onboarding__error">
            <strong>Secure storage is unavailable</strong>
            <span>This computer cannot safely store a linked Finite account.</span>
          </div>
        ) : null}

        {deviceLinkStatus.status === "failed" || error ? (
          <div className="desktop-onboarding__error">
            <strong>Device link</strong>
            <span>{deviceLinkStatus.status === "failed" ? deviceLinkStatus.message : error}</span>
          </div>
        ) : null}
      </section>
    </div>
  );
}
