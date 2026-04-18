import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { vi } from "vitest";
import { NewTabPicker } from "./index";
import type { Tab } from "../TabBar";

describe("Tab type", () => {
  it("accepts terminal type", () => {
    const tab: Tab = { id: "1", title: "Terminal", type: "terminal" };
    expect(tab.type).toBe("terminal");
  });

  it("accepts database type with connection id", () => {
    const tab: Tab = { id: "2", title: "DB", type: "database", dbConnectionId: "conn-1" };
    expect(tab.type).toBe("database");
    expect(tab.dbConnectionId).toBe("conn-1");
  });
});

describe("NewTabPicker", () => {
  it("renders two options when open", () => {
    render(<NewTabPicker onSelect={() => {}} onClose={() => {}} />);
    expect(screen.getByText("終端機")).toBeInTheDocument();
    expect(screen.getByText("資料庫")).toBeInTheDocument();
  });

  it("calls onSelect with terminal when 終端機 clicked", () => {
    const onSelect = vi.fn();
    render(<NewTabPicker onSelect={onSelect} onClose={() => {}} />);
    fireEvent.click(screen.getByText("終端機"));
    expect(onSelect).toHaveBeenCalledWith("terminal");
  });

  it("calls onSelect with database when 資料庫 clicked", () => {
    const onSelect = vi.fn();
    render(<NewTabPicker onSelect={onSelect} onClose={() => {}} />);
    fireEvent.click(screen.getByText("資料庫"));
    expect(onSelect).toHaveBeenCalledWith("database");
  });

  it("calls onClose when Escape pressed", () => {
    const onClose = vi.fn();
    render(<NewTabPicker onSelect={() => {}} onClose={onClose} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
