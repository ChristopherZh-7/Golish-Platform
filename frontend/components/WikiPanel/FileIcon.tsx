import { FileCode2, FileText } from "lucide-react";
import { cn } from "@/lib/utils";
import { isMarkdown } from "./utils";

export function FileIcon({ name, className }: { name: string; className?: string }) {
  if (isMarkdown(name)) return <FileText className={cn("text-blue-400/60", className)} />;
  return <FileCode2 className={cn("text-emerald-400/60", className)} />;
}
