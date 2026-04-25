import { AlertTriangle, CheckCircle2, Circle, Loader2, XCircle } from "lucide-react";
import { cn } from "@/lib/utils";

export function StatusIcon({
  status,
  size = "md",
}: {
  status: string;
  size?: "sm" | "md";
}) {
  const sizeClass = size === "sm" ? "w-3 h-3" : "w-4 h-4";

  switch (status) {
    case "completed":
      return <CheckCircle2 className={cn(sizeClass, "text-[var(--ansi-green)]")} />;
    case "running":
      return <Loader2 className={cn(sizeClass, "text-[var(--ansi-blue)] animate-spin")} />;
    case "error":
      return <XCircle className={cn(sizeClass, "text-[var(--ansi-red)]")} />;
    case "interrupted":
      return <AlertTriangle className={cn(sizeClass, "text-amber-400")} />;
    default:
      return <Circle className={cn(sizeClass, "text-muted-foreground/60")} />;
  }
}
