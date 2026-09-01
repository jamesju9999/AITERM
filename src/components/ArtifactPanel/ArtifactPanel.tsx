import type { ReactNode } from "react";
import { FileTextIcon, ChartIcon } from "../Icons";
import { useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactHtmlFrame } from "./ArtifactHtmlFrame";
import { ArtifactChart, type ChartSpec } from "./ArtifactChart";
import "./ArtifactPanel.css";

export function ArtifactPanel() {
  const { activeArtifact, clearArtifact } = useArtifactPanel();
  if (!activeArtifact) return null;

  let body: ReactNode;
  if (activeArtifact.kind === "html") {
    body = <ArtifactHtmlFrame html={activeArtifact.content} title={activeArtifact.title} />;
  } else {
    let spec: ChartSpec | null = null;
    try {
      spec = JSON.parse(activeArtifact.content) as ChartSpec;
    } catch {
      spec = null;
    }
    body = spec ? (
      <ArtifactChart spec={spec} />
    ) : (
      <div className="aiterm-artifact-panel__error">圖表資料格式錯誤，無法解析。</div>
    );
  }

  return (
    <div className="aiterm-artifact-panel">
      <div className="aiterm-artifact-panel__header">
        {activeArtifact.kind === "html" ? <FileTextIcon size={15} /> : <ChartIcon size={15} />}
        <span className="aiterm-artifact-panel__title">{activeArtifact.title}</span>
        <button
          type="button"
          className="aiterm-artifact-panel__close"
          onClick={clearArtifact}
          title="關閉文件面板"
        >
          ✕
        </button>
      </div>
      <div className="aiterm-artifact-panel__body">{body}</div>
    </div>
  );
}
