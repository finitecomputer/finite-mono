import type { ReactNode } from "react";
import { CheckCircle2Icon, CircleAlertIcon, LoaderCircleIcon } from "lucide-react";

import { cn } from "@/lib/utils";

type ConnectionState = "connected" | "disconnected" | "loading";

export function ConnectionCard({
  account,
  children,
  description,
  error,
  footer,
  icon,
  name,
  state,
}: {
  account?: string | null;
  children: ReactNode;
  description: string;
  error?: string | null;
  footer?: ReactNode;
  icon: ReactNode;
  name: string;
  state: ConnectionState;
}) {
  const status = connectionStatus(state);
  const StatusIcon = status.icon;
  return (
    <section className="ocean-connection-card">
      <div className="ocean-connection-card__main">
        <div className="ocean-connection-card__identity">
          <span className="ocean-connection-card__icon">{icon}</span>
          <div className="min-w-0">
            <h2 className="ocean-connection-card__name">{name}</h2>
            <div
              className={cn(
                "ocean-connection-card__status",
                state === "connected" && "is-connected",
                state === "disconnected" && "is-disconnected"
              )}
            >
              <StatusIcon className="size-4" />
              <span>{status.label}</span>
            </div>
            {account ? <p className="ocean-connection-card__account">{account}</p> : null}
            <p className="mt-2 max-w-xl text-sm leading-6 text-muted-foreground">{description}</p>
            {error ? <p className="mt-2 text-sm text-destructive">{error}</p> : null}
          </div>
        </div>
        <div className="ocean-connection-card__action">{children}</div>
      </div>
      {footer ? <div className="ocean-connection-card__footer">{footer}</div> : null}
    </section>
  );
}

function connectionStatus(state: ConnectionState) {
  if (state === "connected") return { icon: CheckCircle2Icon, label: "Connected" };
  if (state === "disconnected") return { icon: CircleAlertIcon, label: "Not connected" };
  return { icon: LoaderCircleIcon, label: "Checking…" };
}
