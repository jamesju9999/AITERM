import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({}) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));
vi.mock("../../ipc/config", () => ({ getConfig: vi.fn().mockResolvedValue({ submit_shortcut: "enter" }) }));

import { CrossDbAiChat } from "./CrossDbAiChat";

const ARTIFACT_TEXT =
  "報告：\n\n```artifact-html\n<!DOCTYPE html><html><head><title>D</title></head><body>x</body></html>\n```";

describe("CrossDbAiChat artifact wiring", () => {
  beforeEach(() => localStorage.clear());

  it("mounts inside an ArtifactSplit so artifacts have somewhere to render", () => {
    const { container } = render(<CrossDbAiChat databases={[]} />);
    // 沒有 artifact 時分割是「不啟用」狀態，但容器必須在——這就是接線本身。
    expect(container.querySelector(".aiterm-artifact-split")).not.toBeNull();
    expect(container.querySelector(".aiterm-artifact-panel")).toBeNull();
  });

  it("closes the document panel when switching to a history entry that has none", async () => {
    localStorage.setItem("aiterm-crossdb-chat-sessions", JSON.stringify([
      { id: "a", title: "有文件的對話", messages: [{ role: "assistant", text: ARTIFACT_TEXT }], savedAt: 1 },
      { id: "b", title: "純文字對話", messages: [{ role: "assistant", text: "這裡只有文字。" }], savedAt: 2 },
    ]));

    let container: HTMLElement;
    await act(async () => {
      ({ container } = render(<CrossDbAiChat databases={[]} />));
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /歷史/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByText("有文件的對話"));
    });
    expect(container!.querySelector(".aiterm-artifact-panel")).not.toBeNull();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /歷史/ }));
    });
    await act(async () => {
      fireEvent.click(screen.getByText("純文字對話"));
    });
    expect(container!.querySelector(".aiterm-artifact-panel")).toBeNull();
  });
});
