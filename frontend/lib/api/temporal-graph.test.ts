import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./client", () => ({ invoke: vi.fn() }));

import { invoke } from "./client";
import {
  globalSanitizedGraphScope,
  knowledgeGraphQueryScoped,
  knowledgeGraphRebuildScope,
  organizationGraphScope,
} from "./temporal-graph";

const mockedInvoke = vi.mocked(invoke);

describe("temporal graph API", () => {
  beforeEach(() => mockedInvoke.mockReset());

  it("queries only through the closed organization scope request", async () => {
    mockedInvoke.mockResolvedValueOnce({ entities: [], relations: [] });
    const scope = organizationGraphScope("project-1", "organization-1");

    await knowledgeGraphQueryScoped({
      scope,
      query: "host:10.0.0.5",
      validAt: null,
      limit: 100,
    });

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_graph_query_scoped", {
      request: {
        scope: {
          scopeKind: "organization",
          projectScopeId: "project-1",
          organizationIdAtTime: "organization-1",
        },
        query: "host:10.0.0.5",
        validAt: null,
        limit: 100,
      },
    });
  });

  it("rebuilds global-sanitized scope without caller-selected actor authority", async () => {
    mockedInvoke.mockResolvedValueOnce({
      generationId: "generation-1",
      scopeKey: "global_sanitized",
      projectionSchemaVersion: 1,
      status: "active",
      buildHash: "a".repeat(64),
      entityCount: 1,
      relationCount: 0,
    });

    await knowledgeGraphRebuildScope({ scope: globalSanitizedGraphScope() });

    expect(mockedInvoke).toHaveBeenCalledWith("knowledge_graph_rebuild_scope", {
      request: { scope: { scopeKind: "global_sanitized" } },
    });
    const payload = mockedInvoke.mock.calls[0]?.[1];
    expect(payload).not.toHaveProperty("actorId");
    expect(payload).not.toHaveProperty("projectPath");
  });
});
