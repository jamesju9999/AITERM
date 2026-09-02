import { useEffect } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

vi.mock("../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("./i18n")>("./i18n");
  return {
    useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }),
  };
});

import { ArtifactPanelProvider, useArtifactPanel } from "../contexts/ArtifactPanelContext";
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
    // 早期版本在這裡退回原始碼區塊，後來改成「產生中」的卡（見下一條測試）；
    // 這條守的是重點：串流中不可以登記進面板。
    expect(container.querySelector("code.language-artifact-html")).toBeNull();
  });

  // 串流中不渲染卡片是對的（半成品不該進面板），但原本會退回把半成品的原始
  // HTML 直接倒進泡泡——一份報告好幾千個 token，畫面上就是一大坨原始碼，或者
  // 看起來像卡住了。改成顯示一張「產生中」的卡。
  it("shows a pending card instead of raw source while an artifact streams", () => {
    const text = "```artifact-html\n<title>Brief</title><p>hi</p>";
    const { container } = render(
      <ArtifactPanelProvider>
        <MarkdownText text={text} streaming />
      </ArtifactPanelProvider>,
    );
    expect(container.querySelector(".aiterm-artifact-pending")).not.toBeNull();
    expect(container.querySelector("code.language-artifact-html")).toBeNull();
    // 仍然不可以登記進面板——那是完成後的事。
    expect(container.querySelector(".aiterm-artifact-card")).toBeNull();
  });

  it("shows a pending card for a streaming chart too", () => {
    const text = '```artifact-chart\n{"title":"Sales"';
    const { container } = render(
      <ArtifactPanelProvider>
        <MarkdownText text={text} streaming />
      </ArtifactPanelProvider>,
    );
    expect(container.querySelector(".aiterm-artifact-pending")).not.toBeNull();
  });

  // 實機回報的症狀：模型寫了一份含 ``` 的長 HTML 報告，後半段整坨原始 HTML
  // 溢出到聊天泡泡裡。這條守的是「泡泡裡不可以有文件內容」。
  //
  // 注意這條「抓不到」文件被截斷那半個 bug——改成在第一個收尾 fence 截斷，它
  // 一樣會綠（實測過）。文件完整性由 artifactFence.test.ts 的
  // 「keeps going past a fence that appears inside the document」守，那條在
  // 截斷版本下會確實變紅。
  it("does not leak the document into the bubble when it contains a fence", () => {
    const text = [
      "報告如下：",
      "```artifact-html",
      "<title>Report</title>",
      "<h1>前半</h1>",
      "```",
      "<h2>SPILLED</h2>",
      "```",
    ].join("\n");
    const { container } = renderWithProvider(text);
    expect(container.querySelector(".aiterm-artifact-card")).not.toBeNull();
    // 泡泡裡不該看得到文件內容——它屬於面板。
    expect(container.textContent).not.toContain("SPILLED");
    expect(container.textContent).not.toContain("<h2>");
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

  // 這條守的是 markdown.tsx 裡 `components` 的 useMemo。react-markdown 會把
  // components 物件裡的每個函式當成該節點的 React element type：物件若是每次
  // render 都重新建立的字面量，任何祖先重新 render 都會讓 code 節點被卸載重掛，
  // ArtifactBlockCard 的掛載 effect 因此再次呼叫 showArtifact，而那又讓訂閱
  // context 的祖先重新 render——無窮迴圈。
  //
  // 這個迴圈實際發生過（整合 ChatPanelShell 時 vitest 直接卡死）。卡死是最難
  // 診斷的失敗模式，所以這裡用「同一份內容在祖先重新 render 後不該重新註冊」
  // 把它轉成一個會立刻失敗的斷言。
  it("does not re-register the artifact when an ancestor re-renders with unchanged content", () => {
    const text = "```artifact-html\n<title>Brief</title>\n```";
    const registrations: unknown[] = [];

    function RegistrationProbe() {
      const { activeArtifact } = useArtifactPanel();
      useEffect(() => {
        if (activeArtifact) registrations.push(activeArtifact);
      }, [activeArtifact]);
      return null;
    }

    const tree = (tick: string) => (
      <ArtifactPanelProvider>
        <RegistrationProbe />
        <MarkdownText text={text} />
        <span>{tick}</span>
      </ArtifactPanelProvider>
    );

    const { rerender } = render(tree("a"));
    expect(registrations).toHaveLength(1);

    // 只有無關的兄弟節點變了，markdown 內容一模一樣——不該有第二次註冊。
    rerender(tree("b"));
    expect(registrations).toHaveLength(1);
  });
});

// MarkdownText 也被 DesignView / DatabaseAiChat 使用，那兩個地方這個里程碑還
// 沒有掛 ArtifactPanelProvider。在這個功能之前，那裡出現 artifact fence 只是
// 渲染成普通程式碼區塊；如果現在改成直接拋例外，等於這個分支讓那兩個畫面整個
// 崩掉——那是回歸，不是「尚未支援」。沒有 provider 時就退回原本的程式碼區塊。
describe("MarkdownText without an ArtifactPanelProvider", () => {
  it("falls back to a plain code block instead of throwing", () => {
    const text = "```artifact-html\n<title>Brief</title>\n```";
    const { container } = render(<MarkdownText text={text} />);
    expect(container.querySelector("code.language-artifact-html")).not.toBeNull();
    expect(container.querySelector(".aiterm-artifact-card")).toBeNull();
  });
});
