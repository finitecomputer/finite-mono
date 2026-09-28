import type { ReactNode } from "react";
import { CheckCircle2Icon, CircleAlertIcon, LoaderCircleIcon } from "lucide-react";

import { cn } from "@/lib/utils";

type ConnectionState = "connected" | "disconnected" | "attention" | "loading" | "unavailable";

export function ConnectionCard({
  account,
  children,
  description,
  error,
  footer,
  icon,
  name,
  state,
  statusLabel,
  testId,
}: {
  account?: string | null;
  children?: ReactNode;
  description: ReactNode;
  error?: string | null;
  footer?: ReactNode;
  icon: ReactNode;
  name: string;
  state: ConnectionState;
  /** Replaces the default label for `state`, e.g. "Configured" instead of "Connected". */
  statusLabel?: string;
  testId?: string;
}) {
  const status = connectionStatus(state);
  const StatusIcon = status.icon;
  return (
    <section className="ocean-connection-card" data-testid={testId}>
      <div className="ocean-connection-card__main">
        <div className="ocean-connection-card__identity">
          <span className="ocean-connection-card__icon">{icon}</span>
          <div className="min-w-0">
            <h2 className="ocean-connection-card__name">{name}</h2>
            <div
              className={cn(
                "ocean-connection-card__status",
                state === "connected" && "is-connected",
                (state === "disconnected" || state === "unavailable") && "is-disconnected",
                state === "attention" && "is-attention"
              )}
              data-testid={testId ? `${testId}-state` : undefined}
            >
              <StatusIcon className="size-4" />
              <span>{statusLabel ?? status.label}</span>
            </div>
            {account ? <p className="ocean-connection-card__account">{account}</p> : null}
            <p className="mt-2 max-w-xl text-sm leading-6 text-muted-foreground">{description}</p>
            {error ? <p className="mt-2 text-sm text-destructive">{error}</p> : null}
          </div>
        </div>
        {children ? <div className="ocean-connection-card__action">{children}</div> : null}
      </div>
      {footer ? <div className="ocean-connection-card__footer">{footer}</div> : null}
    </section>
  );
}

function connectionStatus(state: ConnectionState) {
  if (state === "connected") return { icon: CheckCircle2Icon, label: "Connected" };
  if (state === "disconnected") return { icon: CircleAlertIcon, label: "Not connected" };
  if (state === "unavailable") return { icon: CircleAlertIcon, label: "Status unavailable" };
  if (state === "attention") return { icon: CircleAlertIcon, label: "Needs attention" };
  return { icon: LoaderCircleIcon, label: "Checking…" };
}
