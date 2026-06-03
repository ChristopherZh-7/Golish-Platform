import { describe, expect, it } from "vitest";
import { prettyStageName } from "./StageMarker";

describe("prettyStageName", () => {
  it("maps known harness stage ids to readable labels", () => {
    expect(prettyStageName("scoping")).toBe("Scoping");
    expect(prettyStageName("external_attack_surface")).toBe("External Attack Surface");
    expect(prettyStageName("vuln_triage")).toBe("Vulnerability Triage");
    expect(prettyStageName("post_exploitation")).toBe("Post-Exploitation");
  });

  it("title-cases unknown ids as a fallback", () => {
    expect(prettyStageName("custom_extra_stage")).toBe("Custom Extra Stage");
    expect(prettyStageName("plain")).toBe("Plain");
  });
});
