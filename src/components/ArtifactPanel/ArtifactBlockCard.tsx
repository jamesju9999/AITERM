import { useEffect, useRef } from "react";
import { nanoid } from "nanoid";
import { FileTextIcon, ChartIcon } from "../Icons";
import { useArtifactPanel, type ArtifactKind } from "../../contexts/ArtifactPanelContext";
import "./ArtifactBlockCard.css";

interface ArtifactBlockCardProps {
  kind: ArtifactKind;
  content: string;
}

function guessTitle(kind: ArtifactKind, content: string): string {
  if (kind === "chart") {
    try {
      const parsed = JSON.parse(content) as { title?: string };
      if (typeof parsed.title === "string" && parsed.title.trim()) return parsed.title.trim();
    } catch {
      // 不是合法 JSON，落到下面的預設標題。
    }
    return "圖表";
  }
  const match = content.match(/<title>([^<]*)<\/title>/i);
  return match?.[1]?.trim() || "文件";
}

/**
 * 聊天泡泡裡代表一個 artifact 區塊的精簡卡片。掛載時透過 useEffect（不能在
 * 渲染期間呼叫，React 不允許在渲染一個元件時觸發另一個元件的狀態更新）把
 * 內容登記進 ArtifactPanelContext，讓側邊的 ArtifactPanel 顯示出來；點卡片
 * 可以重新叫出來（例如使用者切走去看別的 artifact 之後想切回來）。
 */
export function ArtifactBlockCard({ kind, content }: ArtifactBlockCardProps) {
  const { showArtifact } = useArtifactPanel();
  const idRef = useRef(nanoid());
  const title = guessTitle(kind, content);

  useEffect(() => {
    showArtifact({ id: idRef.current, kind, title, content });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, content, title]);

  return (
    <button
      type="button"
      className="aiterm-artifact-card"
      onClick={() => showArtifact({ id: idRef.current, kind, title, content })}
    >
      {kind === "html" ? <FileTextIcon size={14} /> : <ChartIcon size={14} />}
      <span className="aiterm-artifact-card__title">{title}</span>
      <span className="aiterm-artifact-card__kind">{kind === "html" ? "HTML" : "Chart"}</span>
    </button>
  );
}
