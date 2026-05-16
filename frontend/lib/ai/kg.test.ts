/**
 * Unit tests for KG SDK wrappers.
 *
 * The Tauri commands themselves are validated in the backend test
 * suite; here we cover the typed thin layer + the pure helper
 * `groupEntitiesByType` that downstream UI panels will rely on.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import * as client from "@/lib/api/client";
import {
  groupEntitiesByType,
  kgGetNeighbors,
  type KgEntity,
  kgListEntities,
  kgSearchEntities,
} from "./kg";

vi.mock("@/lib/api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api/client")>();
  return {
    ...actual,
    invoke: vi.fn(),
  };
});

const sample = (override: Partial<KgEntity>): KgEntity => ({
  id: "11111111-1111-4111-8111-111111111111",
  entity_type: "host",
  name: "10.0.0.5",
  properties: {},
  project_id: null,
  created_at: "2026-05-17T00:00:00Z",
  updated_at: "2026-05-17T00:00:00Z",
  ...override,
});

describe("kg SDK wrappers", () => {
  beforeEach(() => {
    (client.invoke as unknown as ReturnType<typeof vi.fn>).mockReset();
  });

  it("kgListEntities forwards projectId/entityType/limit to invoke()", async () => {
    (client.invoke as any).mockResolvedValue([sample({})]);
    const out = await kgListEntities({
      projectId: "/tmp/proj",
      entityType: "host",
      limit: 50,
    });
    expect(out).toHaveLength(1);
    expect(client.invoke).toHaveBeenCalledWith("kg_list_entities", {
      projectId: "/tmp/proj",
      entityType: "host",
      limit: 50,
    });
  });

  it("kgListEntities defaults to projectId=null when omitted", async () => {
    (client.invoke as any).mockResolvedValue([]);
    await kgListEntities();
    expect(client.invoke).toHaveBeenCalledWith("kg_list_entities", {
      projectId: null,
      entityType: undefined,
      limit: undefined,
    });
  });

  it("kgSearchEntities forwards query + options", async () => {
    (client.invoke as any).mockResolvedValue([]);
    await kgSearchEntities("CVE-2024-1234", { entityType: "vulnerability", limit: 10 });
    expect(client.invoke).toHaveBeenCalledWith("kg_search_entities", {
      query: "CVE-2024-1234",
      entityType: "vulnerability",
      limit: 10,
    });
  });

  it("kgGetNeighbors forwards entityId + optional relationType", async () => {
    (client.invoke as any).mockResolvedValue([]);
    await kgGetNeighbors("abc-uuid", "has_vulnerability");
    expect(client.invoke).toHaveBeenCalledWith("kg_get_neighbors", {
      entityId: "abc-uuid",
      relationType: "has_vulnerability",
    });
  });
});

describe("groupEntitiesByType", () => {
  it("buckets entities by their entity_type field", () => {
    const ents: KgEntity[] = [
      sample({ entity_type: "host", name: "10.0.0.5" }),
      sample({ entity_type: "host", name: "10.0.0.6" }),
      sample({ entity_type: "vulnerability", name: "CVE-2024-1234" }),
    ];
    const grouped = groupEntitiesByType(ents);
    expect(grouped.host).toHaveLength(2);
    expect(grouped.vulnerability).toHaveLength(1);
    expect(Object.keys(grouped).sort()).toEqual(["host", "vulnerability"]);
  });

  it("preserves order within each bucket", () => {
    const ents: KgEntity[] = [
      sample({ entity_type: "host", name: "A" }),
      sample({ entity_type: "host", name: "B" }),
      sample({ entity_type: "host", name: "C" }),
    ];
    const grouped = groupEntitiesByType(ents);
    expect(grouped.host.map((e) => e.name)).toEqual(["A", "B", "C"]);
  });

  it("returns an empty object for an empty input", () => {
    expect(groupEntitiesByType([])).toEqual({});
  });

  it("handles non-canonical entity_type strings", () => {
    const grouped = groupEntitiesByType([
      sample({ entity_type: "asset" as any }),
      sample({ entity_type: "host" }),
    ]);
    expect(grouped.asset).toHaveLength(1);
    expect(grouped.host).toHaveLength(1);
  });
});
