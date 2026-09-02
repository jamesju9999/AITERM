import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({}) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

import { CrossDbAiChat } from "./CrossDbAiChat";

describe("CrossDbAiChat artifact wiring", () => {
  it("mounts inside an ArtifactSplit so artifacts have somewhere to render", () => {
    const { container } = render(<CrossDbAiChat databases={[]} />);
    // 沒有 artifact 時分割是「不啟用」狀態，但容器必須在——這就是接線本身。
    expect(container.querySelector(".aiterm-artifact-split")).not.toBeNull();
    expect(container.querySelector(".aiterm-artifact-panel")).toBeNull();
  });
});
