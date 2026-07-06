import { DashboardShell } from "@/components/dashboard-shell";

export function AppShell() {
  return (
    <DashboardShell
      machines={[]}
      isAdmin={false}
      saasMode={false}
    >
      <div className="rounded-[1.5rem] border border-border/60 bg-card p-6 shadow-sm">
        <p className="text-sm text-muted-foreground">
          Use the route-based dashboard surfaces instead of this scaffold entrypoint.
        </p>
      </div>
    </DashboardShell>
  );
}
