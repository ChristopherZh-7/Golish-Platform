import { memo, useMemo } from "react";
import { cn } from "@/lib/utils";

export interface GitDiffLine {
  oldLineNum: number | null;
  newLineNum: number | null;
  content: string;
  type: "header" | "hunk" | "add" | "remove" | "context";
}

/**
 * Parse a git-style unified diff (with `diff`, `index`, `---`, `+++`, `@@` headers)
 * into structured lines, including old/new line numbers tracked through hunks.
 *
 * Compared to the simpler `parseDiff` used by `DiffView`, this preserves header / hunk
 * lines and computes per-line numbers so the renderer can show line-number gutters.
 */
export function parseGitDiff(diffText: string): GitDiffLine[] {
  const lines = diffText.split("\n");
  const result: GitDiffLine[] = [];
  let oldLine = 0;
  let newLine = 0;

  for (const line of lines) {
    if (
      line.startsWith("diff ") ||
      line.startsWith("index ") ||
      line.startsWith("---") ||
      line.startsWith("+++")
    ) {
      result.push({ oldLineNum: null, newLineNum: null, content: line, type: "header" });
    } else if (line.startsWith("@@")) {
      const match = line.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      if (match) {
        oldLine = parseInt(match[1], 10);
        newLine = parseInt(match[2], 10);
      }
      result.push({ oldLineNum: null, newLineNum: null, content: line, type: "hunk" });
    } else if (line.startsWith("+")) {
      result.push({ oldLineNum: null, newLineNum: newLine, content: line, type: "add" });
      newLine++;
    } else if (line.startsWith("-")) {
      result.push({ oldLineNum: oldLine, newLineNum: null, content: line, type: "remove" });
      oldLine++;
    } else {
      result.push({
        oldLineNum: oldLine,
        newLineNum: newLine,
        content: line || " ",
        type: "context",
      });
      oldLine++;
      newLine++;
    }
  }

  return result;
}

export interface GitDiffViewProps {
  /** Raw unified diff text including `diff`/`index`/`---`/`+++`/`@@` headers. */
  content: string;
}

/**
 * Git-style diff renderer with old/new line-number gutters and hunk/header markers.
 * Used by the Git panel for staging review.
 *
 * For the lightweight tool-output / AI-response variant (no headers, no line numbers),
 * use the sibling `DiffView` component instead.
 */
export const GitDiffView = memo(function GitDiffView({ content }: GitDiffViewProps) {
  const lines = useMemo(() => parseGitDiff(content), [content]);
  const lineNumWidth = useMemo(() => {
    const maxLine = lines.reduce(
      (max, l) => Math.max(max, l.oldLineNum ?? 0, l.newLineNum ?? 0),
      0
    );
    return Math.max(3, String(maxLine).length);
  }, [lines]);

  return (
    <div className="text-xs font-mono">
      {lines.map((line, i) => {
        let lineClass = "text-muted-foreground";
        let bgClass = "";
        let indicator = " ";
        let indicatorClass = "";

        if (line.type === "add") {
          lineClass = "text-emerald-400";
          bgClass = "bg-emerald-400/10";
          indicator = "+";
          indicatorClass = "text-emerald-400";
        } else if (line.type === "remove") {
          lineClass = "text-red-400";
          bgClass = "bg-red-400/10";
          indicator = "-";
          indicatorClass = "text-red-400";
        } else if (line.type === "hunk") {
          lineClass = "text-sky-400";
        } else if (line.type === "header") {
          lineClass = "text-muted-foreground font-semibold";
        }

        const showLineNums = line.type !== "header" && line.type !== "hunk";
        const displayContent =
          line.type === "add" || line.type === "remove" ? line.content.slice(1) : line.content;

        return (
          <div
            key={`${line.type}-${line.oldLineNum ?? "n"}-${line.newLineNum ?? "n"}-${i}`}
            className="flex"
          >
            {showLineNums ? (
              <>
                <span
                  className="text-muted-foreground/50 select-none px-1 text-right shrink-0"
                  style={{ width: `${lineNumWidth + 1}ch` }}
                >
                  {line.oldLineNum ?? ""}
                </span>
                <span
                  className="text-muted-foreground/50 select-none px-1 text-right shrink-0"
                  style={{ width: `${lineNumWidth + 1}ch` }}
                >
                  {line.newLineNum ?? ""}
                </span>
                <span
                  className={cn(
                    "select-none w-4 text-center shrink-0 border-r border-border",
                    indicatorClass
                  )}
                >
                  {indicator}
                </span>
              </>
            ) : (
              <span
                className="shrink-0 border-r border-border"
                style={{ width: `${(lineNumWidth + 1) * 2 + 2}ch` }}
              />
            )}
            <span className={cn("whitespace-pre flex-1 pl-2", lineClass, bgClass)}>
              {displayContent}
            </span>
          </div>
        );
      })}
    </div>
  );
});
