"use client";

import { useActionState, type ReactNode } from "react";
import { openBillingPortalAction } from "@/app/actions";
import { Button } from "@/components/ui/button";

export function ManageBillingForm({
  className,
  variant = "default",
  children = "Manage billing",
}: {
  className?: string;
  variant?: "default" | "outline";
  children?: ReactNode;
}) {
  const [state, action, pending] = useActionState(openBillingPortalAction, {});
  return <form action={action} className={className}>
    <Button variant={variant} disabled={pending}>{pending ? "Opening…" : children}</Button>
    {state.error && <p role="alert" className="mt-2 text-sm text-destructive">{state.error}</p>}
  </form>;
}
