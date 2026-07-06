import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export function StickerPageHeader({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <header className="border-b border-border bg-card/60 px-6 py-8">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="type-label text-muted-foreground">Development</p>
        {action}
      </div>
      <h1 className="mt-1 type-title-1">{title}</h1>
      <p className="mt-2 max-w-2xl type-body-lg text-muted-foreground">{description}</p>
    </header>
  );
}

export function StickerBlock({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("mb-10 last:mb-0", className)}>
      <h3 className="mb-3 type-label text-muted-foreground">{title}</h3>
      {children}
    </section>
  );
}

export function StickerRow({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("flex flex-wrap items-center gap-3", className)}>{children}</div>;
}
