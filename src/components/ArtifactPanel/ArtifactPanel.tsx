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
 * 上炸掉、把整個面板帶走。渲染前先驗形狀。
 *
 * 失敗時回傳「哪裡不對」而不只是 null——只說一句「格式錯誤」的話，使用者跟
 * 維護者都只能猜模型到底寫了什麼。
 */
function parseChartSpec(raw: string): { spec: ChartSpec } | { problem: string } {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (e) {
    return { problem: `JSON 解析失敗：${e instanceof Error ? e.message : String(e)}` };
  }
  if (typeof parsed !== "object" || parsed === null) {
    return { problem: "最外層不是一個物件。" };
  }
  const s = parsed as Partial<ChartSpec>;
  const missing: string[] = [];
  if (s.type !== "bar" && s.type !== "line" && s.type !== "pie") {
    missing.push('type（必須是 "bar" / "line" / "pie"）');
  }
  if (!Array.isArray(s.data)) missing.push("data（必須是陣列）");
  if (typeof s.xKey !== "string" || s.xKey.length === 0) missing.push("xKey（必須是非空字串）");
  if (
    !Array.isArray(s.series) ||
    s.series.length === 0 ||
    !s.series.every((e) => e && typeof e.key === "string" && typeof e.label === "string")
  ) {
    missing.push("series（必須是 [{ key, label }] 且不可為空）");
  }
  return missing.length === 0
    ? { spec: parsed as ChartSpec }
    : { problem: `欄位不符：${missing.join("、")}` };
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
    const result = parseChartSpec(activeArtifact.content);
    body = "spec" in result ? (
      <ArtifactChart spec={result.spec} />
    ) : (
      <div className="aiterm-artifact-panel__error">
        <p>圖表資料格式錯誤，無法解析。</p>
        <p className="aiterm-artifact-panel__error-why">{result.problem}</p>
        <pre className="aiterm-artifact-panel__error-raw">{activeArtifact.content}</pre>
      </div>
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
