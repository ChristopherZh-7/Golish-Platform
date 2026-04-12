import { memo } from "react";
import type { ActiveSubAgent } from "@/store";
import { TaskGroupShell } from "@/components/TaskGroupShell";
import { SubAgentCard } from "./SubAgentCard";

interface SubAgentGroupProps {
  agents: ActiveSubAgent[];
}

export const SubAgentGroup = memo(function SubAgentGroup({ agents }: SubAgentGroupProps) {
  if (agents.length === 0) return null;

  const running = agents.filter((a) => a.status === "running").length;
  const completed = agents.filter((a) => a.status === "completed").length;
  const errored = agents.filter((a) => a.status === "error").length;
  const totalDurationMs = agents.reduce((sum, a) => sum + (a.durationMs ?? 0), 0);

  return (
    <TaskGroupShell
      title="Agent Task Group"
      running={running}
      completed={completed}
      failed={errored}
      total={agents.length}
      totalDurationMs={totalDurationMs}
    >
      <div className="px-2 py-1">
        {agents.map((agent) => (
          <SubAgentCard
            key={agent.parentRequestId}
            subAgent={agent}
            autoCollapse={agent.status === "completed"}
          />
        ))}
      </div>
    </TaskGroupShell>
  );
});
