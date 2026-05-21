/**
 * Tests for [`CaptureButton`]: conditional render + onStart wiring.
 *
 * Tooltip / TooltipProvider come from Radix Popper which doesn't need
 * special setup under jsdom, but we don't assert on tooltip *content*
 * (only the trigger button) to keep tests deterministic across Radix
 * versions.
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { IntegrationGroup as IntegrationGroupSchema } from "@/lib/api/integrations";
import { CaptureButton } from "./CaptureButton";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (k: string) => k,
  }),
}));

const BASE_GROUP: IntegrationGroupSchema = {
  id: "aqc",
  name: "AQC",
  fields: [
    {
      key: "cookies.aqc",
      label: "Cookie",
      type: "secret_textarea",
      required: true,
    },
  ],
};

describe("<CaptureButton>", () => {
  it("renders nothing when group.capture is absent", () => {
    const { container } = render(
      <CaptureButton toolId="enscan-go" group={BASE_GROUP} onStart={() => {}} />
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders the button when group.capture is present", () => {
    const group: IntegrationGroupSchema = {
      ...BASE_GROUP,
      capture: {
        login_url: "https://aiqicha.baidu.com",
        timeout_secs: 300,
        rules: [
          {
            type: "cookie",
            domain: ".aiqicha.baidu.com",
            name: "BDUSS",
            target_field: "cookies.aqc",
          },
        ],
      },
    };
    render(<CaptureButton toolId="enscan-go" group={group} onStart={() => {}} />);
    expect(
      screen.getByRole("button", { name: "integrations.capture.button.label" })
    ).toBeInTheDocument();
  });

  it("calls onStart with (toolId, groupId) when clicked", async () => {
    const onStart = vi.fn();
    const user = userEvent.setup();
    const group: IntegrationGroupSchema = {
      ...BASE_GROUP,
      capture: {
        login_url: "https://aiqicha.baidu.com",
        timeout_secs: 300,
        rules: [],
      },
    };
    render(<CaptureButton toolId="enscan-go" group={group} onStart={onStart} />);
    await user.click(
      screen.getByRole("button", { name: "integrations.capture.button.label" })
    );
    expect(onStart).toHaveBeenCalledTimes(1);
    expect(onStart).toHaveBeenCalledWith("enscan-go", "aqc");
  });

  it("does not fire onStart when disabled", async () => {
    const onStart = vi.fn();
    const user = userEvent.setup();
    const group: IntegrationGroupSchema = {
      ...BASE_GROUP,
      capture: {
        login_url: "https://aiqicha.baidu.com",
        timeout_secs: 300,
        rules: [],
      },
    };
    render(
      <CaptureButton
        toolId="enscan-go"
        group={group}
        disabled
        onStart={onStart}
      />
    );
    await user.click(
      screen.getByRole("button", { name: "integrations.capture.button.label" })
    );
    expect(onStart).not.toHaveBeenCalled();
  });
});
