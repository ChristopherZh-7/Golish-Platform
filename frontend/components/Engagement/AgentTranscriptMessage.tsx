import { ThinkingBlock } from "@/components/AIChatPanel/ThinkingBlock";
import { Markdown } from "@/components/Markdown";
import type { ActiveSubAgent, SubAgentEntry } from "@/store";

export interface AgentTranscriptMessageProps {
  kind: "text" | "thinking";
  actorLabel: string;
  text: string;
  timeLabel?: string;
  startedAt?: number;
  endedAt?: number;
  thinkingActive?: boolean;
}

/**
 * A reasoning entry's `endedAt` is the latest chunk timestamp, not a semantic
 * completion marker. Only the tail reasoning entry of a running Agent is live;
 * the next text/tool entry settles it.
 */
export function isLiveAgentThinkingEntry(
  activity: Pick<ActiveSubAgent, "status" | "entries">,
  entry: SubAgentEntry
): boolean {
  return (
    entry.kind === "thinking" &&
    activity.status === "running" &&
    activity.entries[activity.entries.length - 1] === entry
  );
}

/** Compact, readable transcript row shared by every unified Stage workspace. */
export function AgentTranscriptMessage({
  kind,
  actorLabel,
  text,
  timeLabel,
  startedAt,
  endedAt,
  thinkingActive = false,
}: AgentTranscriptMessageProps) {
  if (kind === "thinking") {
    return (
      <article
        data-testid="agent-transcript-thinking"
        className="grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2.5 px-4 py-2"
      >
        <div className="col-start-2 min-w-0 max-w-[86ch]">
          <ThinkingBlock
            content={text}
            isActive={thinkingActive}
            startedAt={startedAt}
            endedAt={endedAt}
            variant="detail"
          />
        </div>
      </article>
    );
  }

  return (
    <article
      data-testid="agent-transcript-message"
      className="group grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2.5 px-4 py-3.5"
    >
      <div className="grid h-7 w-7 place-items-center rounded-lg border border-sky-300/10 bg-sky-400/10 text-[10px] font-semibold text-sky-200">
        A
      </div>
      <div className="min-w-0">
        <div className="flex min-h-5 flex-wrap items-center gap-2 text-[11px]">
          <span className="font-semibold text-foreground/90">{actorLabel}</span>
          {timeLabel && <span className="tabular-nums text-muted-foreground/55">{timeLabel}</span>}
        </div>
        <Markdown
          content={text}
          className="mt-1.5 max-w-[86ch] select-text text-[12px] leading-[1.65] text-foreground/88 [overflow-wrap:anywhere] [&_p]:mb-2 [&_p]:leading-[1.65] [&_p]:text-foreground/88 [&_p:last-child]:mb-0 [&_li]:leading-[1.6] [&_li]:text-foreground/88 [&_ul]:my-2 [&_ol]:my-2 [&_strong]:font-semibold [&_code]:whitespace-normal [&_code]:break-words [&_pre]:max-w-full"
        />
      </div>
    </article>
  );
}
