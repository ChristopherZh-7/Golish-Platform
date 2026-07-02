import { describe, expect, it } from "vitest";
import type { Target } from "@/lib/pentest/types";
import { groupTargetsByHost } from "./asset-groups";

function target(id: string, patch: Partial<Target>): Target {
  return {
    id,
    name: id,
    type: "domain",
    value: id,
    scope: "in",
    organization_id: "org-1",
    real_ip: "",
    ports: [],
    ...patch,
  } as Target;
}

describe("groupTargetsByHost", () => {
  it("groups an IP target with domains and URL targets that resolve to it", () => {
    const groups = groupTargetsByHost(
      [
        target("ip", { type: "ip", value: "1.1.1.1" }),
        target("domain", { value: "a.example.com", real_ip: "1.1.1.1" }),
        target("url", {
          type: "url",
          value: "https://a.example.com/login",
          real_ip: "1.1.1.1",
        }),
      ],
      "Unresolved"
    );

    expect(groups).toHaveLength(1);
    expect(groups[0]).toMatchObject({
      label: "1.1.1.1",
      inScope: 3,
      outScope: 0,
    });
    expect(groups[0].ipTarget?.id).toBe("ip");
    expect(groups[0].linkedTargets.map((item) => item.id).sort()).toEqual(["domain", "url"]);
  });

  it("keeps IP rows under their own value even when real_ip carries provider attribution", () => {
    const groups = groupTargetsByHost(
      [
        target("ip-child", { type: "ip", value: "115.223.9.114", real_ip: "124.71.187.144" }),
        target("ip-parent", { type: "ip", value: "124.71.187.144" }),
        target("domain", { value: "dayu.example.com", real_ip: "124.71.187.144" }),
      ],
      "Unresolved"
    );

    expect(groups.map((group) => group.label)).toEqual(["115.223.9.114", "124.71.187.144"]);
    expect(groups[0].targets.map((item) => item.id)).toEqual(["ip-child"]);
    expect(groups[1].targets.map((item) => item.id).sort()).toEqual(["domain", "ip-parent"]);
  });

  it("keeps IP-literal URL targets out of the linked domain display", () => {
    const groups = groupTargetsByHost(
      [
        target("ip", { type: "ip", value: "1.1.1.1" }),
        target("url-ip", { type: "url", value: "https://1.1.1.1/login" }),
      ],
      "Unresolved"
    );

    expect(groups).toHaveLength(1);
    expect(groups[0].targets.map((item) => item.id).sort()).toEqual(["ip", "url-ip"]);
    expect(groups[0].linkedTargets.map((item) => item.id)).toEqual([]);
  });

  it("keeps unresolved domains in a final catch-all group", () => {
    const groups = groupTargetsByHost(
      [
        target("domain", { value: "a.example.com" }),
        target("ip", { type: "ip", value: "2.2.2.2" }),
      ],
      "Unresolved"
    );

    expect(groups.map((group) => group.label)).toEqual(["2.2.2.2", "Unresolved"]);
    expect(groups[1].linkedTargets.map((item) => item.id)).toEqual(["domain"]);
  });

  it("dedupes www aliases in the display list without hiding sibling subdomains", () => {
    const groups = groupTargetsByHost(
      [
        target("ip", { type: "ip", value: "115.28.135.55" }),
        target("mobile", { value: "m.moresec.cn", real_ip: "115.28.135.55" }),
        target("apex", { value: "moresec.cn", real_ip: "115.28.135.55" }),
        target("www", { value: "www.moresec.cn", real_ip: "115.28.135.55" }),
      ],
      "Unresolved"
    );

    expect(groups).toHaveLength(1);
    expect(groups[0].inScope).toBe(4);
    expect(groups[0].targets.map((item) => item.id).sort()).toEqual([
      "apex",
      "ip",
      "mobile",
      "www",
    ]);
    expect(groups[0].linkedTargets.map((item) => item.id).sort()).toEqual(["apex", "mobile"]);
  });
});
