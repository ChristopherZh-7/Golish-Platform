import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AsyncView } from "./AsyncView";

describe("AsyncView", () => {
  it("renders the default spinner while loading", () => {
    const { container } = render(
      <AsyncView loading={true} error={null}>
        <div>content</div>
      </AsyncView>
    );

    expect(container.querySelector(".animate-spin")).toBeInTheDocument();
    expect(screen.queryByText("content")).not.toBeInTheDocument();
  });

  it("renders the error message when error is set", () => {
    render(
      <AsyncView loading={false} error="kaboom">
        <div>content</div>
      </AsyncView>
    );

    expect(screen.getByText("kaboom")).toBeInTheDocument();
    expect(screen.queryByText("content")).not.toBeInTheDocument();
  });

  it("renders the empty fallback when empty", () => {
    render(
      <AsyncView loading={false} error={null} isEmpty emptyMessage="Nothing here">
        <div>content</div>
      </AsyncView>
    );

    expect(screen.getByText("Nothing here")).toBeInTheDocument();
    expect(screen.queryByText("content")).not.toBeInTheDocument();
  });

  it("renders children when loaded, not empty, and no error", () => {
    render(
      <AsyncView loading={false} error={null} isEmpty={false}>
        <div>content</div>
      </AsyncView>
    );

    expect(screen.getByText("content")).toBeInTheDocument();
  });

  it("prefers a custom loading fallback when provided", () => {
    render(
      <AsyncView loading={true} error={null} loadingFallback={<div>custom-loading</div>}>
        <div>content</div>
      </AsyncView>
    );

    expect(screen.getByText("custom-loading")).toBeInTheDocument();
  });

  it("prioritizes error over empty", () => {
    render(
      <AsyncView loading={false} error="bad" isEmpty>
        <div>content</div>
      </AsyncView>
    );

    expect(screen.getByText("bad")).toBeInTheDocument();
  });
});
