import { describe, expect, it } from "vitest";
import { buildHostTree } from "./org-tree";

const org = { id: "o1", name: "Acme", parent_id: null, sort_order: 0 } as any;
const tgt = (over: any) =>
  ({
    id: over.value,
    name: over.value,
    type: "domain",
    value: over.value,
    scope: "in",
    real_ip: "",
    organization_id: "o1",
    ...over,
  }) as any;

describe("buildHostTree", () => {
  it("nests domains under their real_ip host node and bins unresolved", () => {
    const targets = [
      tgt({ value: "1.1.1.1", type: "ip" }),
      tgt({ value: "a.com", real_ip: "1.1.1.1" }),
      tgt({ value: "b.com", real_ip: "" }),
    ];
    const roots = buildHostTree([org], targets, "Unassigned", "Unresolved");
    const acme = roots[0];

    const host = acme.children.find((c) => c.kind === "host" && c.name === "1.1.1.1");
    expect(host).toBeDefined();
    // The IP target itself seeds the host node and the domain resolving to it joins.
    expect(host?.targets.map((x) => x.value).sort()).toEqual(["1.1.1.1", "a.com"]);

    const bucket = acme.children.find((c) => c.kind === "bucket");
    expect(bucket?.targets.map((x) => x.value)).toEqual(["b.com"]);

    // The org's flat target list is emptied — everything moves into host/bucket children.
    expect(acme.targets).toEqual([]);
  });

  it("keeps the org spine and groups under the right org", () => {
    const targets = [tgt({ value: "9.9.9.9", type: "ip" }), tgt({ value: "x.com", real_ip: "9.9.9.9" })];
    const roots = buildHostTree([org], targets, "Unassigned", "Unresolved");
    expect(roots).toHaveLength(1);
    expect(roots[0].id).toBe("o1");
    const host = roots[0].children.find((c) => c.kind === "host");
    expect(host?.name).toBe("9.9.9.9");
    expect(host?.targets.map((x) => x.value).sort()).toEqual(["9.9.9.9", "x.com"]);
  });
});
