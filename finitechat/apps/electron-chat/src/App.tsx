import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChatProduct } from "@finite/chat-ui/react";

import {
  DesktopOnboarding,
  type DesktopDeviceLinkReady,
  type DesktopDeviceLinkStatus,
  type DesktopIdentityStatus,
  type DesktopOnboardingStatus,
} from "./components/desktop-onboarding";
import { desktopChatTransport, resolveDaemonUrl } from "./daemon";

/**
 * Electron owns the native trust bootstrap and nothing inside the signed-in
 * chat product. Once this Device is linked, the exact same ChatProduct used
 * by the dashboard talks to the local daemon through a transport adapter.
 */
export function App() {
  const [daemonUrl, setDaemonUrl] = useState<string | null>(null);
  const [identityStatus, setIdentityStatus] = useState<DesktopIdentityStatus | null>(null);
  const [onboardingStatus, setOnboardingStatus] = useState<DesktopOnboardingStatus | null>(null);
  const [deviceLinkStatus, setDeviceLinkStatus] = useState<DesktopDeviceLinkStatus>({
    status: "idle",
  });
  const [identityBusy, setIdentityBusy] = useState(true);
  const [shellError, setShellError] = useState<string | null>(null);
  const linkedOnboardingHandledRef = useRef(false);
  const lastDesktopTargetUrlRef = useRef<{ url: string; timestamp: number } | null>(null);

  const transport = useMemo(
    () => (daemonUrl ? desktopChatTransport(daemonUrl) : null),
    [daemonUrl]
  );
  const showOnboarding =
    onboardingStatus?.completed !== true || identityStatus?.hasStoredAccountSecret !== true;

  useEffect(() => {
    let cancelled = false;
    resolveDaemonUrl()
      .then((url) => {
        if (!cancelled) setDaemonUrl(url);
      })
      .catch((error: unknown) => {
        if (!cancelled) setShellError(errorMessage(error));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const loadDesktopState = useCallback(async () => {
    const desktop = window.finiteChatDesktop;
    if (!desktop) {
      setShellError("Finite Chat desktop bridge is unavailable");
      setIdentityBusy(false);
      return;
    }
    setIdentityBusy(true);
    try {
      const [identity, onboarding] = await Promise.all([
        desktop.identityStatus(),
        desktop.onboardingStatus(),
      ]);
      setIdentityStatus(identity);
      setOnboardingStatus(onboarding);
      setShellError(null);
    } catch (error) {
      setShellError(errorMessage(error));
    } finally {
      setIdentityBusy(false);
    }
  }, []);

  useEffect(() => {
    void loadDesktopState();
  }, [loadDesktopState]);

  const completeLinkedOnboarding = useCallback(async () => {
    const desktop = window.finiteChatDesktop;
    if (!desktop || linkedOnboardingHandledRef.current) return;
    linkedOnboardingHandledRef.current = true;
    setIdentityBusy(true);
    setShellError(null);
    try {
      const [identity, onboarding] = await Promise.all([
        desktop.identityStatus(),
        desktop.completeOnboarding(),
      ]);
      setIdentityStatus(identity);
      setOnboardingStatus(onboarding);
    } catch (error) {
      linkedOnboardingHandledRef.current = false;
      setShellError(errorMessage(error));
    } finally {
      setIdentityBusy(false);
    }
  }, []);

  useEffect(() => {
    const desktop = window.finiteChatDesktop;
    if (!desktop) return;
    return desktop.onDeviceLinkStatus((status) => {
      if (status.status !== "linked") linkedOnboardingHandledRef.current = false;
      setDeviceLinkStatus(status);
      if (status.status === "waiting") {
        setIdentityStatus((current) => (current ? { ...current, linking: true } : current));
      } else if (status.status === "cancelled" || status.status === "failed") {
        setIdentityStatus((current) => (current ? { ...current, linking: false } : current));
      }
    });
  }, []);

  useEffect(() => {
    if (deviceLinkStatus.status === "linked") void completeLinkedOnboarding();
  }, [completeLinkedOnboarding, deviceLinkStatus.status]);

  useEffect(() => {
    const desktop = window.finiteChatDesktop;
    if (!desktop || !transport || showOnboarding) return;

    const openTarget = (url: string | null | undefined) => {
      const value = url?.trim();
      if (!value) return;
      const previous = lastDesktopTargetUrlRef.current;
      const now = Date.now();
      if (previous?.url === value && now - previous.timestamp < 2_000) return;
      lastDesktopTargetUrlRef.current = { url: value, timestamp: now };
      void transport.dispatch({ ScanTarget: { value } }).catch((error: unknown) => {
        setShellError(errorMessage(error));
      });
    };

    const unsubscribe = desktop.onTargetUrl(openTarget);
    void desktop.consumePendingTargetUrl().then(openTarget).catch((error: unknown) => {
      setShellError(errorMessage(error));
    });
    return unsubscribe;
  }, [showOnboarding, transport]);

  async function beginDeviceLink() {
    const desktop = window.finiteChatDesktop;
    if (!desktop) return;
    setIdentityBusy(true);
    setShellError(null);
    try {
      const ready = await desktop.beginDeviceLink();
      setDeviceLinkStatus({ status: "waiting", ready });
      setIdentityStatus((current) => (current ? { ...current, linking: true } : current));
    } catch (error) {
      setDeviceLinkStatus({ status: "failed", message: errorMessage(error) });
    } finally {
      setIdentityBusy(false);
    }
  }

  async function openDeviceLinkApproval(ready: DesktopDeviceLinkReady) {
    const opened = await window.finiteChatDesktop?.openDeviceLinkApproval(ready.approval_url);
    if (opened === false) setShellError("Could not open the approval page. Try again.");
  }

  async function cancelDeviceLink() {
    const desktop = window.finiteChatDesktop;
    if (!desktop) return;
    setIdentityBusy(true);
    try {
      await desktop.cancelDeviceLink();
      setDeviceLinkStatus({ status: "cancelled" });
      setIdentityStatus((current) => (current ? { ...current, linking: false } : current));
    } catch (error) {
      setShellError(errorMessage(error));
    } finally {
      setIdentityBusy(false);
    }
  }

  async function clearDesktopIdentity() {
    const desktop = window.finiteChatDesktop;
    if (!desktop) return;
    setIdentityBusy(true);
    setShellError(null);
    try {
      const identity = await desktop.clearAccountSecret();
      setIdentityStatus(identity);
      setOnboardingStatus({ completed: false });
      setDeviceLinkStatus({ status: "idle" });
      linkedOnboardingHandledRef.current = false;
    } catch (error) {
      setShellError(errorMessage(error));
    } finally {
      setIdentityBusy(false);
    }
  }

  return (
    <div className="desktop-chat-shell">
      {!showOnboarding && transport ? (
        <ChatProduct
          transport={transport}
          machineLabel="Finite Chat"
          navigation={{
            signOut: {
              label: "Remove account from this Mac",
              onClick: clearDesktopIdentity,
            },
          }}
        />
      ) : null}

      {!showOnboarding && shellError ? (
        <div className="desktop-chat-shell__error" role="alert">
          {shellError}
        </div>
      ) : null}

      {showOnboarding ? (
        <DesktopOnboarding
          busy={identityBusy}
          deviceLinkStatus={deviceLinkStatus}
          error={shellError}
          identityStatus={identityStatus}
          onBeginLink={() => void beginDeviceLink()}
          onCancelLink={() => void cancelDeviceLink()}
          onOpenApproval={(ready) => void openDeviceLinkApproval(ready)}
          onUseLinkedAccount={() => void completeLinkedOnboarding()}
        />
      ) : null}
    </div>
  );
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
