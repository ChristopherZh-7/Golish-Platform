import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./client", () => ({ invoke: vi.fn() }));

import { invoke } from "./client";
import {
  type InvestigationRequestStopRequest,
  investigationRequestStop,
} from "./investigation";

const mockedInvoke = vi.mocked(invoke);

describe("Investigation API", () => {
  beforeEach(() => mockedInvoke.mockReset());

  it("sends only the exact generated stop selector and CAS authority", async () => {
    mockedInvoke.mockResolvedValueOnce(undefined);
    const request = {
      sessionId: "session-1",
      operationId: "11111111-1111-4111-8111-111111111111",
      stageExecutionId: "22222222-2222-4222-8222-222222222222",
      stageRunRequestId: "investigation-stage-run-1",
      expectedInvestigationRunStateHead: `sha256:${"a".repeat(64)}`,
      expectedChangeSeq: 7,
      idempotencyKey: "33333333-3333-4333-8333-333333333333",
    } satisfies InvestigationRequestStopRequest;

    await investigationRequestStop(request);

    expect(mockedInvoke).toHaveBeenCalledWith("investigation_request_stop", {
      request,
    });
    expect(mockedInvoke.mock.calls[0]?.[1]).not.toHaveProperty("workIds");
    expect(mockedInvoke.mock.calls[0]?.[1]).not.toHaveProperty("principalId");
  });
});
