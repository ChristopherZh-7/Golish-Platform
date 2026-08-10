import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "@/store";
import { SubAgentInlineCard } from "./SubAgentInlineCard";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const SESSION_ID = "sub-agent-inline-stage";
const STAGE_REQUEST_ID = "stage-request";
const AGENT_REQUEST_ID = `${STAGE_REQUEST_ID}::team::org-1::worker:worker-1`;

function seedSession(withStage = true) {
  useStore.setState({
    sessions: {
      [SESSION_ID]: {
        id: SESSION_ID,
        name: "Agent workspace",
        workingDirectory: "/tmp",
        createdAt: "2026-08-02T00:00:00Z",
        mode: "agent",
        detailViewMode: "timeline",
        stageRuns: withStage
          ? {
              [STAGE_REQUEST_ID]: {
                requestId: STAGE_REQUEST_ID,
                stageLabel: "Enumeration",
                roleLabel: "Company Controller",
                coverageAxis: [],
                rows: [],
                summary: { total: 0, covered: 0, active: 0, queued: 0, blocked: 0 },
              },
            }
          : undefined,
      },
    },
    timelines: { [SESSION_ID]: [] },
    activeSubAgents: {},
  });
}

describe("SubAgentInlineCard detail routing", () => {
  beforeEach(() => seedSession());

  it("opens the owning Stage workspace and preserves the exact Agent focus", () => {
    render(
      <SubAgentInlineCard
        requestId={AGENT_REQUEST_ID}
        sessionId={SESSION_ID}
        toolCall={{ name: "sub_agent_js", args: '{"task":"analyze JS"}' }}
      />
    );

    fireEvent.click(screen.getByRole("button"));

    expect(useStore.getState().sessions[SESSION_ID]).toEqual(
      expect.objectContaining({
        detailViewMode: "tool-detail",
        toolDetailRequestIds: [STAGE_REQUEST_ID, AGENT_REQUEST_ID],
      })
    );
  });

  it("fails closed to the timeline when no owning Stage or tool execution exists", () => {
    seedSession(false);
    render(
      <SubAgentInlineCard
        requestId="orphan-agent"
        sessionId={SESSION_ID}
        toolCall={{ name: "sub_agent_js", args: "{}" }}
      />
    );

    fireEvent.click(screen.getByRole("button"));

    expect(useStore.getState().sessions[SESSION_ID]).toEqual(
      expect.objectContaining({
        detailViewMode: "timeline",
        toolDetailRequestIds: null,
      })
    );
  });

  it("recovers the owning Stage from a persisted historical Agent identity", () => {
    seedSession(false);
    useStore.setState((state) => {
      state.timelines[SESSION_ID] = [
        {
          id: "historical-stage-tool",
          type: "ai_tool_execution",
          timestamp: "2026-08-02T00:00:00Z",
          data: {
            requestId: STAGE_REQUEST_ID,
            toolName: "stage_run",
            args: { stage: "enumeration" },
            status: "completed",
            startedAt: "2026-08-02T00:00:00Z",
          },
        },
      ];
    });

    render(
      <SubAgentInlineCard
        requestId={AGENT_REQUEST_ID}
        sessionId={SESSION_ID}
        toolCall={{ name: "sub_agent_js", args: "{}" }}
      />
    );
    fireEvent.click(screen.getByRole("button"));

    expect(useStore.getState().sessions[SESSION_ID]?.toolDetailRequestIds).toEqual([
      STAGE_REQUEST_ID,
      AGENT_REQUEST_ID,
    ]);
    expect(useStore.getState().sessions[SESSION_ID]?.detailViewMode).toBe("tool-detail");
  });
});
