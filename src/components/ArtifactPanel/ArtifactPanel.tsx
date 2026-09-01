import { useCallback, type ReactNode } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { FileTextIcon, ChartIcon, DownloadIcon } from "../Icons";
import { writeTextFile } from "../../ipc/fs";
import { useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactHtmlFrame } from "./ArtifactHtmlFrame";
import { ArtifactChart, type ChartSpec } from "./ArtifactChart";
import "./ArtifactPanel.css";

/**
 * JSON.parse 只保證「是合法 JSON」，不保證「是 ChartSpec」。內容是模型寫的，
 * 少一個 series 或整個給 {} 都很常見，而 ArtifactChart 會直接在 spec.series.length
 * 上炸掉、把整個面板帶走。渲染前先驗形狀，不合格就當成解析失敗處理。
 */
function parseChartSpec(raw: string): ChartSpec | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const s = parsed as Partial<ChartSpec>;
  const typeOk = s.type === "bar" || s.type === "line" || s.type === "pie";
  const dataOk = Array.isArray(s.data);
  const xKeyOk = typeof s.xKey === "string" && s.xKey.length > 0;
  const seriesOk =
    Array.isArray(s.series) &&
    s.series.length > 0 &&
    s.series.every((e) => e && typeof e.key === "string" && typeof e.label === "string");
  return typeOk && dataOk && xKeyOk && seriesOk ? (parsed as ChartSpec) : null;
}

/** 標題直接來自模型寫的 <title>，可能含有路徑分隔字元等不能當檔名的東西。 */
function toFileName(title: string): string {
  const cleaned = title.replace(/[\\/:*?"<>|]/g, "_").trim();
  return `${cleaned || "document"}.html`;
}

export function ArtifactPanel() {
  const { activeArtifact, clearArtifact } = useArtifactPanel();

  const isHtml = activeArtifact?.kind === "html";
  const html = activeArtifact?.content ?? "";
  const title = activeArtifact?.title ?? "";

  const handleDownload = useCallback(async () => {
    const path = await save({
      defaultPath: toFileName(title),
      filters: [{ name: "HTML", extensions: ["html"] }],
    });
    if (!path) return;
    await writeTextFile(path, html);
  }, [title, html]);

  if (!activeArtifact) return null;

  let body: ReactNode;
  if (activeArtifact.kind === "html") {
    body = <ArtifactHtmlFrame html={activeArtifact.content} title={activeArtifact.title} />;
  } else {
    const spec = parseChartSpec(activeArtifact.content);
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
        {isHtml && (
          <button
            type="button"
            className="aiterm-artifact-panel__action"
            onClick={handleDownload}
            title="下載 HTML 文件"
          >
            <DownloadIcon size={14} />
          </button>
        )}
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
