import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

vi.mock("../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("./i18n")>("./i18n");
  return {
    useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }),
  };
});

import { ArtifactPanelProvider } from "../contexts/ArtifactPanelContext";
import { MarkdownText } from "./markdown";
import { MessageBubble } from "../components/AiPanel/MessageBubble";

function renderWithProvider(text: string) {
  return render(
    <ArtifactPanelProvider>
      <MarkdownText text={text} />
    </ArtifactPanelProvider>,
  );
}

describe("MarkdownText artifact fenced blocks", () => {
  it("renders a complete artifact-html fenced block as a card, not raw code", () => {
    const text = "before\n\n```artifact-html\n<title>Brief</title><p>hi</p>\n```\n\nafter";
    renderWithProvider(text);
    expect(screen.getByText("Brief")).toBeInTheDocument();
    expect(screen.queryByText(/<title>/)).not.toBeInTheDocument();
  });

  it("renders a complete artifact-chart fenced block as a card", () => {
    const text = '```artifact-chart\n{"title":"Sales"}\n```';
    renderWithProvider(text);
    expect(screen.getByText("Sales")).toBeInTheDocument();
  });

  // 串流中不要渲染 artifact 卡片：CommonMark 對未閉合的 fence 會在輸入結束時
  // 隱式閉合並產出完整的 code node（實測過，不是理論），所以「等 fence 收完」
  // 這件事不會自己發生。真正可靠的訊號是 MessageList 傳給串流中氣泡的
  // streaming prop——完成的訊息不帶這個 prop，卡片就在那時才出現、只註冊一次。
  it("does not render a card while the message is still streaming", () => {
    const text = "```artifact-html\n<title>Brief</title><p>hi</p>\n```";
    const { container } = render(
      <ArtifactPanelProvider>
        <MarkdownText text={text} streaming />
      </ArtifactPanelProvider>,
    );
    expect(container.querySelector(".aiterm-artifact-card")).toBeNull();
    expect(container.querySelector("code.language-artifact-html")).not.toBeNull();
  });

  it("a plain mermaid block still renders inline as before (regression check)", () => {
    const text = '```mermaid\npie title x\n"a" : 1\n```';
    const { container } = renderWithProvider(text);
    expect(container.querySelector(".aiterm-artifact-card")).toBeNull();
  });

  it("MessageBubble does not render a card for a streaming assistant message, but does once finished", () => {
    const text = "```artifact-html\n<title>Brief</title>\n```";
    const { container, rerender } = render(
      <ArtifactPanelProvider>
        <MessageBubble role="assistant" content={text} onExecuteCommand={() => {}} streaming />
      </ArtifactPanelProvider>,
    );
    expect(container.querySelector(".aiterm-artifact-card")).toBeNull();

    rerender(
      <ArtifactPanelProvider>
        <MessageBubble role="assistant" content={text} onExecuteCommand={() => {}} />
      </ArtifactPanelProvider>,
    );
    expect(container.querySelector(".aiterm-artifact-card")).not.toBeNull();
  });
});
