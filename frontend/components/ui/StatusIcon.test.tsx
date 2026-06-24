import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatusIcon } from "./StatusIcon";

describe("StatusIcon", () => {
  it("keeps a fixed flex size in dense tool rows", () => {
    const { container } = render(<StatusIcon status="completed" size="sm" />);

    const className = container.querySelector("svg")?.getAttribute("class") ?? "";

    expect(className).toContain("w-3");
    expect(className).toContain("h-3");
    expect(className).toContain("shrink-0");
  });
});
