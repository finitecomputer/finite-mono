import * as React from "react";

import { cn } from "@/lib/utils";

export function Kbd({
  className,
  ...props
}: React.ComponentProps<"kbd">) {
  return (
    <kbd
      className={cn(
        "inline-flex h-6 min-w-6 items-center justify-center rounded-md border border-border/70 bg-muted px-1.5 text-[11px] font-medium text-muted-foreground shadow-sm",
        className
      )}
      {...props}
    />
  );
}

export function KbdGroup({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span className={cn("inline-flex items-center gap-1", className)} {...props} />
  );
}
