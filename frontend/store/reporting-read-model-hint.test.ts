import { beforeEach, describe, expect, it } from "vitest";
import { useStore } from "./index";

const SESSION_ID = "reporting-hint-session";

describe("Reporting read-model session hint", () => {
  beforeEach(() => {
    useStore.setState({
      sessions: {
        [SESSION_ID]: {
          id: SESSION_ID,
          name: "Reporting",
          workingDirectory: "/tmp/reporting",
          createdAt: "2026-07-13T00:00:00Z",
          mode: "agent",
        },
      },
    });
  });

  it("increments refreshes for one operation and replaces identity with a fresh version", () => {
    const store = useStore.getState();
    store.setReportingReadModelHint(SESSION_ID, { operationId: "operation-1" });
    store.setReportingReadModelHint(SESSION_ID, { operationId: "operation-1" });
    expect(useStore.getState().sessions[SESSION_ID].reportingReadModelHint).toEqual({
      operationId: "operation-1",
      refreshVersion: 2,
    });

    store.setReportingReadModelHint(SESSION_ID, { operationId: "operation-2" });
    expect(useStore.getState().sessions[SESSION_ID].reportingReadModelHint).toEqual({
      operationId: "operation-2",
      refreshVersion: 1,
    });
  });

  it("can clear a stale hint", () => {
    const store = useStore.getState();
    store.setReportingReadModelHint(SESSION_ID, { operationId: "operation-1" });
    store.setReportingReadModelHint(SESSION_ID, null);
    expect(useStore.getState().sessions[SESSION_ID].reportingReadModelHint).toBeUndefined();
  });
});
