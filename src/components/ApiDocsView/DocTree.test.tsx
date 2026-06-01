import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DocTree } from "./DocTree";
import type { DocNode } from "../../ipc/apiDocs";

const tree: DocNode[] = [
  {
    title: "Getting Started",
    href: "/docs/getting-started",
    items: [
      { title: "Quickstart", href: "/docs/quickstart", items: [] },
    ],
  },
  { title: "API Reference", href: "/docs/api", items: [] },
];

describe("DocTree", () => {
  it("renders all leaf nodes", () => {
    render(
      <DocTree nodes={tree} selected={new Set()} onChange={() => {}} filter="" />
    );
    expect(screen.getByText("Getting Started")).toBeInTheDocument();
    expect(screen.getByText("Quickstart")).toBeInTheDocument();
    expect(screen.getByText("API Reference")).toBeInTheDocument();
  });

  it("calls onChange when a leaf is checked", () => {
    const onChange = vi.fn();
    render(
      <DocTree nodes={tree} selected={new Set()} onChange={onChange} filter="" />
    );
    const checkbox = screen.getByLabelText("Quickstart");
    fireEvent.click(checkbox);
    expect(onChange).toHaveBeenCalled();
    const newSet: Set<string> = onChange.mock.calls[0][0];
    expect(newSet.has("/docs/quickstart")).toBe(true);
  });

  it("hides non-matching nodes when filter is set", () => {
    render(
      <DocTree nodes={tree} selected={new Set()} onChange={() => {}} filter="quick" />
    );
    expect(screen.queryByText("API Reference")).not.toBeInTheDocument();
    expect(screen.getByText("Quickstart")).toBeInTheDocument();
  });
});
