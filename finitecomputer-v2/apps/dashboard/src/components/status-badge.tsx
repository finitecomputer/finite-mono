import { Badge } from "@/components/ui/badge";
import type { StatusBadgeState } from "@/lib/design-tokens";
import { cn } from "@/lib/utils";

export function StatusBadge({
  status,
}: {
  status: StatusBadgeState;
}) {
  const tone = statusBadgeTone(status);

  return (
    <Badge
      variant="outline"
      className={cn(
        "rounded-full px-2.5 py-0.5 capitalize",
        tone === "emerald" && "border-emerald-300 bg-emerald-50 text-emerald-700",
        tone === "amber" && "border-amber-300 bg-amber-50 text-amber-700",
        tone === "rose" && "border-rose-300 bg-rose-50 text-rose-700",
        tone === "zinc" && "border-zinc-300 bg-zinc-50 text-zinc-700"
      )}
    >
      {status.replace("_", " ")}
    </Badge>
  );
}

function statusBadgeTone(status: StatusBadgeState) {
  switch (status) {
    case "complete":
      return "emerald";
    case "in_progress":
      return "amber";
    case "blocked":
      return "rose";
    default:
      return "zinc";
  }
}
