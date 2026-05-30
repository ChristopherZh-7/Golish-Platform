import { describe, expect, it } from "vitest";
import type { PortInfo } from "@/lib/pentest/types";
import { formatTime, isHttpPort } from "./surfaceModel";

const port = (p: Partial<PortInfo>): PortInfo => p as PortInfo;

describe("isHttpPort", () => {
  it("detects http via service name", () => {
    expect(isHttpPort(port({ service: "http" }))).toBe(true);
    expect(isHttpPort(port({ service: "https" }))).toBe(true);
  });

  it("detects http via status or title even when service is not http", () => {
    expect(isHttpPort(port({ service: "ssh", http_status: 200 }))).toBe(true);
    expect(isHttpPort(port({ service: "ssh", http_title: "Home" }))).toBe(true);
  });

  it("returns false for non-web services", () => {
    expect(isHttpPort(port({ service: "ssh" }))).toBe(false);
    expect(isHttpPort(port({}))).toBe(false);
  });
});

describe("formatTime", () => {
  it("echoes the raw value when unparseable", () => {
    expect(formatTime("not-a-date")).toBe("not-a-date");
  });

  it("formats a parseable ISO string into a clock time", () => {
    // locale-tolerant: assert HH:MM:SS appears (AM/PM suffix allowed)
    expect(formatTime("2026-01-01T13:05:09Z")).toMatch(/\d{2}:\d{2}:\d{2}/);
  });

  it("accepts epoch millis", () => {
    expect(formatTime(Date.parse("2026-01-01T13:05:09Z"))).toMatch(/\d{2}:\d{2}:\d{2}/);
  });
});
