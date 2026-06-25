import { AlertTriangle, CheckCircle2, Circle, Clock, Loader2, XCircle } from "lucide-react";
import { cn } from "@/lib/utils";

export function StatusIcon({
  status,
  size = "md",
  className,
}: {
  status: string;
  size?: "sm" | "md";
  className?: string;
}) {
  const sizeClass = size === "sm" ? "w-3 h-3" : "w-4 h-4";
  const iconClass = cn(sizeClass, "shrink-0", className);

  switch (status) {
    case "completed":
      return <CheckCircle2 className={cn(iconClass, "text-[var(--ansi-green)]")} />;
    case "running":
      return <Loader2 className={cn(iconClass, "text-[var(--ansi-blue)] animate-spin")} />;
    case "backgrounded":
      return <Clock className={cn(iconClass, "text-amber-300 animate-pulse")} />;
    case "error":
      return <XCircle className={cn(iconClass, "text-[var(--ansi-red)]")} />;
    case "interrupted":
      return <AlertTriangle className={cn(iconClass, "text-amber-400")} />;
    default:
      return <Circle className={cn(iconClass, "text-muted-foreground/60")} />;
  }
}
