import { useRef, useState, type PointerEvent, type ReactNode } from "react";
import { useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactPanel } from "./ArtifactPanel";
import "./ArtifactSplit.css";

const MIN_CHAT_COLUMN_WIDTH = 220;
const MIN_ARTIFACT_COLUMN_WIDTH = 260;

interface ArtifactSplitProps {
  /** 聊天欄的內容。沒有 artifact 時它佔滿整個容器。 */
  children: ReactNode;
}

/**
 * 有 artifact 時把版面裂成「聊天欄 + 可拖拉分隔線 + ArtifactPanel」，沒有時就
 * 只是一個透明的容器。自帶樣式，所以宿主用 CSS class 還是 inline style 都能接。
 *
 * 抽成共用元件而不是各介面各寫一份，是因為這裡的拖曳有一個只有實機才會發現的
 * 陷阱：右欄是 iframe，游標一進到它的範圍，window 上的 mousemove 就變成 iframe
 * 自己那份文件的事件、父視窗完全收不到，拖曳會一頓一頓。必須用
 * setPointerCapture 把該 pointer 的後續事件強制導回分隔線。複製五份幾乎保證
 * 後四份會重蹈覆轍。
 */
export function ArtifactSplit({ children }: ArtifactSplitProps) {
  const { activeArtifact } = useArtifactPanel();
  const [chatColumnWidth, setChatColumnWidth] = useState(320);
  const [isResizing, setIsResizing] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const isDraggingRef = useRef(false);

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    isDraggingRef.current = true;
    setIsResizing(true);
    (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!isDraggingRef.current || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    setChatColumnWidth(Math.max(
      MIN_CHAT_COLUMN_WIDTH,
      Math.min(e.clientX - rect.left, rect.width - MIN_ARTIFACT_COLUMN_WIDTH),
    ));
  };

  const onPointerUp = () => {
    if (!isDraggingRef.current) return;
    isDraggingRef.current = false;
    setIsResizing(false);
  };

  const className = [
    "aiterm-artifact-split",
    activeArtifact ? "aiterm-artifact-split--active" : "",
    isResizing ? "aiterm-artifact-split--resizing" : "",
  ].filter(Boolean).join(" ");

  return (
    <div className={className} ref={containerRef}>
      <div
        className="aiterm-artifact-split__chat"
        style={activeArtifact
          ? { width: `${chatColumnWidth}px`, flexShrink: 0, flexGrow: 0 }
          : { flex: 1 }}
      >
        {children}
      </div>

      {activeArtifact && (
        <>
          <div
            className="aiterm-artifact-resizer"
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            title="拖曳調整寬度"
          />
          <ArtifactPanel />
        </>
      )}
    </div>
  );
}
