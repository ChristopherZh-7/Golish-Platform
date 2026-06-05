import { describe, expect, it } from "vitest";
import { stripAgentXmlTags } from "./SubAgentDetailView";

describe("stripAgentXmlTags", () => {
  it("removes empty <tool_call></tool_call> wrappers", () => {
    expect(stripAgentXmlTags("<tool_call>\n</tool_call>")).toBe("");
  });

  it("removes a complete tool_call block with an inner function", () => {
    const text =
      "before\n<tool_call>\n<function=graph_search>\n<parameter=query>example.com</parameter>\n</function>\n</tool_call>\nafter";
    const out = stripAgentXmlTags(text);
    expect(out).not.toContain("<tool_call>");
    expect(out).not.toContain("</tool_call>");
    expect(out).not.toContain("<function=");
    expect(out).toContain("before");
    expect(out).toContain("after");
  });

  it("removes a lone/unterminated trailing <tool_call> tag", () => {
    const text = "Let me also check the evidence directory for more files.<tool_call>";
    const out = stripAgentXmlTags(text);
    expect(out).not.toContain("<tool_call>");
    expect(out).toContain("Let me also check the evidence directory for more files.");
  });

  it("still strips function/parameter and context tags", () => {
    const text =
      "<task_assignment>do x</task_assignment>\n<function=dns_resolve>\n<parameter=domain>example.com</parameter>\n</function>";
    const out = stripAgentXmlTags(text);
    expect(out).not.toContain("<function=");
    expect(out).not.toContain("<parameter=");
    expect(out).not.toContain("task_assignment");
  });

  it("leaves plain prose untouched", () => {
    expect(stripAgentXmlTags("just a normal answer")).toBe("just a normal answer");
  });
});
