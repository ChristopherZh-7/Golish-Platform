/**
 * Integration-level test for [`IntegrationGroupForm`]: it should
 * render exactly one input per declared field, drive the
 * `integrations.set` IPC on Save, and disable Save while required
 * fields are blank.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock the IPC layer (must be done before importing the component).
const getMock = vi.fn();
const setMock = vi.fn();
const clearMock = vi.fn();
const testMock = vi.fn();
const captureClearProfileMock = vi.fn();
vi.mock("@/lib/api", () => ({
  integrations: {
    listSchemas: vi.fn(),
    get: (...args: unknown[]) => getMock(...args),
    set: (...args: unknown[]) => setMock(...args),
    clear: (...args: unknown[]) => clearMock(...args),
    test: (...args: unknown[]) => testMock(...args),
    captureClearProfile: (...args: unknown[]) => captureClearProfileMock(...args),
  },
}));
vi.mock("@/lib/notify", () => ({
  notify: {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
    warning: vi.fn(),
  },
}));
// Stub react-i18next so `t(key)` returns the key (deterministic in tests).
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) =>
      typeof opts?.defaultValue === "string" ? (opts.defaultValue as string) : key,
  }),
}));

import type { IntegrationGroup as IntegrationGroupSchema } from "@/lib/api/integrations";
import { IntegrationGroupForm } from "./IntegrationGroup";

const TYC_GROUP: IntegrationGroupSchema = {
  id: "tyc",
  name: "TYC",
  fields: [
    {
      key: "cookies.tyc",
      label: "Cookie",
      type: "secret_textarea",
      required: true,
      rows: 4,
    },
    { key: "tyc.tycid", label: "tycid", type: "secret_text", required: true },
    { key: "tyc.auth_token", label: "auth_token", type: "secret_text", required: true },
  ],
  test: { kind: "exec", cmd: "{{exec}} -n test -type tyc", ok_regex: "company_name" },
};

const EMPTY_SNAPSHOT = {
  "cookies.tyc": { has_value: false },
  "tyc.tycid": { has_value: false },
  "tyc.auth_token": { has_value: false },
};

describe("IntegrationGroupForm", () => {
  beforeEach(() => {
    getMock.mockReset();
    setMock.mockReset();
    clearMock.mockReset();
    testMock.mockReset();
    captureClearProfileMock.mockReset();
    getMock.mockResolvedValue(EMPTY_SNAPSHOT);
    setMock.mockResolvedValue(undefined);
    captureClearProfileMock.mockResolvedValue(undefined);
    vi.stubGlobal("confirm", vi.fn(() => true));
  });

  afterEach(() => {
    vi.clearAllMocks();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("renders exactly one input per declared field", async () => {
    render(<IntegrationGroupForm toolId="enscan-go" group={TYC_GROUP} />);

    await waitFor(() => {
      expect(screen.getByText("Cookie")).toBeInTheDocument();
    });
    expect(screen.getByText("tycid")).toBeInTheDocument();
    expect(screen.getByText("auth_token")).toBeInTheDocument();

    // 1 textarea (cookie) + 2 password inputs (tycid + auth_token) = 3 inputs total.
    const textareas = document.querySelectorAll("textarea");
    expect(textareas.length).toBe(1);
    const passwordInputs = document.querySelectorAll("input[type='password']");
    expect(passwordInputs.length).toBe(2);
  });

  it("disables Save while required fields are blank, enables it after typing", async () => {
    const user = userEvent.setup();
    render(<IntegrationGroupForm toolId="enscan-go" group={TYC_GROUP} />);
    await waitFor(() => expect(screen.getByText("Cookie")).toBeInTheDocument());

    const saveBtn = screen.getByRole("button", { name: /integrations\.save/i });
    expect(saveBtn).toBeDisabled();

    // Fill all 3 required fields.
    const textareas = document.querySelectorAll("textarea");
    const passwords = document.querySelectorAll("input[type='password']");
    await user.type(textareas[0] as HTMLTextAreaElement, "tyc-cookie-blob");
    await user.type(passwords[0] as HTMLInputElement, "tyc-id-value");
    await user.type(passwords[1] as HTMLInputElement, "tyc-auth-token");

    expect(saveBtn).not.toBeDisabled();
  });

  it("calls integrations.set with the typed values on Save", async () => {
    const user = userEvent.setup();
    render(<IntegrationGroupForm toolId="enscan-go" group={TYC_GROUP} />);
    await waitFor(() => expect(screen.getByText("Cookie")).toBeInTheDocument());

    const textareas = document.querySelectorAll("textarea");
    const passwords = document.querySelectorAll("input[type='password']");
    await user.type(textareas[0] as HTMLTextAreaElement, "cookie-blob");
    await user.type(passwords[0] as HTMLInputElement, "id123");
    await user.type(passwords[1] as HTMLInputElement, "token123");

    const saveBtn = screen.getByRole("button", { name: /integrations\.save/i });
    await user.click(saveBtn);

    await waitFor(() => expect(setMock).toHaveBeenCalledTimes(1));
    expect(setMock).toHaveBeenCalledWith({
      toolId: "enscan-go",
      groupId: "tyc",
      fields: {
        "cookies.tyc": "cookie-blob",
        "tyc.tycid": "id123",
        "tyc.auth_token": "token123",
      },
    });
  });

  it("hides the Test button when the group has no test recipe", async () => {
    const noTest: IntegrationGroupSchema = { ...TYC_GROUP, test: undefined };
    render(<IntegrationGroupForm toolId="enscan-go" group={noTest} />);
    await waitFor(() => expect(screen.getByText("Cookie")).toBeInTheDocument());
    expect(screen.queryByRole("button", { name: /integrations\.testButton/i })).toBeNull();
  });

  it("clears the capture browser login profile without clearing stored credentials", async () => {
    const user = userEvent.setup();
    const group: IntegrationGroupSchema = {
      ...TYC_GROUP,
      capture: {
        login_url: "https://example.com/login",
        timeout_secs: 300,
        rules: [],
      },
    };
    render(<IntegrationGroupForm toolId="enscan-go" group={group} />);
    await waitFor(() => expect(screen.getByText("Cookie")).toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: /integrations\.capture\.clearProfile\.label/i }));

    await waitFor(() => expect(captureClearProfileMock).toHaveBeenCalledTimes(1));
    expect(captureClearProfileMock).toHaveBeenCalledWith({
      toolId: "enscan-go",
      groupId: "tyc",
    });
    expect(clearMock).not.toHaveBeenCalled();
  });

  it("surfaces an error banner when integrations_get rejects", async () => {
    getMock.mockRejectedValueOnce(new Error("backend down"));
    render(<IntegrationGroupForm toolId="enscan-go" group={TYC_GROUP} />);
    await waitFor(() => {
      // "integrations.loadFailed: backend down" — the key plus the
      // rejected error message.
      expect(
        screen.getByText((c) => c.includes("integrations.loadFailed") && c.includes("backend down"))
      ).toBeInTheDocument();
    });
  });
});
