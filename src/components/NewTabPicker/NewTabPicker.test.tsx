import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { vi } from "vitest";
import { NewTabPicker } from "./index";
import type { Tab } from "../TabBar";
import { LocaleProvider } from "../../contexts/LocaleContext";

beforeEach(() => {
  Object.defineProperty(window, "localStorage", {
    value: { getItem: vi.fn(() => null), setItem: vi.fn(), removeItem: vi.fn(), clear: vi.fn(), key: vi.fn(), length: 0 },
    writable: true,
  });
});

function renderPicker(onSelect = vi.fn(), onClose = vi.fn()) {
  return render(
    <LocaleProvider>
      <NewTabPicker onSelect={onSelect} onClose={onClose} />
    </LocaleProvider>
  );
}

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
    renderPicker();
    expect(screen.getByText("終端機")).toBeInTheDocument();
    expect(screen.getByText("資料庫")).toBeInTheDocument();
  });

  it("calls onSelect with terminal when 終端機 clicked", () => {
    const onSelect = vi.fn();
    renderPicker(onSelect);
    fireEvent.click(screen.getByText("終端機"));
    expect(onSelect).toHaveBeenCalledWith("terminal");
  });

  it("calls onSelect with database when 資料庫 clicked", () => {
    const onSelect = vi.fn();
    renderPicker(onSelect);
    fireEvent.click(screen.getByText("資料庫"));
    expect(onSelect).toHaveBeenCalledWith("database");
  });

  it("calls onClose when Escape pressed", () => {
    const onClose = vi.fn();
    renderPicker(vi.fn(), onClose);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});
