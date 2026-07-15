// src/components/DocConverter/DocConverterView.tsx
import React, { useState, useRef, useCallback, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { aiChat, formatAiError } from "../../ipc/ai";
import { markitdownConvert, markitdownPickFile } from "../../ipc/markitdown";
import { useLocale } from "../../contexts/LocaleContext";
import "./DocConverterView.css";

interface ExtractState {
  fileName: string;
  rawText: string;
}

const CHUNK_SIZE = 3500;

const URL_RE = /https?:\/\/[^\s）)]+/g;

function renderErrorWithLinks(msg: string): React.ReactNode {
  const parts: React.ReactNode[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  URL_RE.lastIndex = 0;
  while ((m = URL_RE.exec(msg)) !== null) {
    if (m.index > last) parts.push(msg.slice(last, m.index));
    const url = m[0];
    parts.push(
      <a key={m.index} href={url} target="_blank" rel="noopener noreferrer"
        style={{ color: "inherit", textDecoration: "underline" }}>
        {url}
      </a>
    );
    last = m.index + url.length;
  }
  if (last < msg.length) parts.push(msg.slice(last));
  return parts;
}

const NORMALIZATION_SYSTEM_PROMPT = `你是資料字典格式化工具。將輸入的原始文字整理成結構化的 Markdown 格式。

每個資料表輸出：
## TABLE_NAME
一行說明（如果有）

| 欄位名 | 型別 | 說明 |
|--------|------|------|
| 欄位1 | 型別 | 說明文字 |

規則：
1. 每張表必須以 ## 開頭的標題行（## 表名）
2. 欄位資訊放在 3 欄 Markdown 表格（欄位名 | 型別 | 說明）
3. 如果原文件未提供型別，填入 -
4. 只輸出 Markdown，不要加解釋文字或開場白
5. 保留原始的資料表名稱和欄位名稱（不要翻譯）`;

export function DocConverterView({ isActive: _isActive }: { isActive: boolean }) {
  const { t } = useLocale();
  const [extractState, setExtractState] = useState<ExtractState | null>(null);
  const [extracting, setExtracting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [downloadSuccess, setDownloadSuccess] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [mdOutput, setMdOutput] = useState<string>("");
  const [normalizing, setNormalizing] = useState(false);
  const [normalizeProgress, setNormalizeProgress] = useState<{ step: number; total: number } | null>(null);
  const stoppedRef = useRef(false);

  const processFilePath = useCallback(async (filePath: string) => {
    stoppedRef.current = true;
    setNormalizing(false);
    setNormalizeProgress(null);
    setError(null);
    setDownloadSuccess(null);
    setExtractState(null);
    setMdOutput("");
    setExtracting(true);
    try {
      const markdown = await markitdownConvert(filePath, selectedProviderId || undefined);
      const fileName = filePath.split(/[\\/]/).pop() ?? filePath;
      setExtractState({ fileName, rawText: markdown });
    } catch (e) {
      setError(t.dc_extract_error(String(e)));
    } finally {
      setExtracting(false);
    }
  }, [selectedProviderId]);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
  }, []);

  // OS-level drag-drop (Tauri intercepts before the web DOM)
  useEffect(() => {
    type DragDropPayload = { paths: string[]; position?: unknown };
    const unlisten = listen<DragDropPayload>("tauri://drag-drop", async (event) => {
      const paths = event.payload?.paths;
      if (!paths?.length) return;
      processFilePath(paths[0]);
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [processFilePath]);

  const handleDropzoneClick = useCallback(async () => {
    const path = await markitdownPickFile();
    if (path) processFilePath(path);
  }, [processFilePath]);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    // Web-level drag-drop: extract path from DataTransfer (works for some browsers/OS combos)
    // The OS-level event listener above handles the common case on Tauri.
    const item = e.dataTransfer.items[0];
    if (item?.kind === "file") {
      const file = item.getAsFile();
      if (file) {
        // Tauri exposes the real path on File objects in a web context
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const path: string | undefined = (file as any).path;
        if (path) processFilePath(path);
      }
    }
  }, [processFilePath]);

  const normalizeWithAi = useCallback(async () => {
    if (!extractState) return;
    stoppedRef.current = false;
    setError(null);
    setDownloadSuccess(null);
    setMdOutput("");
    setNormalizing(true);

    const text = extractState.rawText;
    const totalChunks = Math.ceil(text.length / CHUNK_SIZE);
    setNormalizeProgress({ step: 0, total: totalChunks });

    const parts: string[] = [];
    for (let i = 0; i < totalChunks; i++) {
      if (stoppedRef.current) break;
      setNormalizeProgress({ step: i + 1, total: totalChunks });
      const chunk = text.slice(i * CHUNK_SIZE, (i + 1) * CHUNK_SIZE);
      try {
        const aiResult = await aiChat(
          [{ role: "system" as const, content: NORMALIZATION_SYSTEM_PROMPT }, { role: "user" as const, content: chunk }],
          "docconverter",
          selectedProviderId || undefined,
        );
        parts.push((aiResult.content ?? "").trim());
      } catch (e) {
        const aiErr = typeof e === "object" && e !== null && "kind" in e
          ? formatAiError(e as import("../../ipc/ai").AiError)
          : String(e);
        setError(t.dc_normalize_error(i + 1, aiErr));
        break;
      }
    }

    setMdOutput(parts.join("\n\n"));
    setNormalizing(false);
    setNormalizeProgress(null);
  }, [extractState, selectedProviderId]);

  const downloadMd = useCallback(() => {
    if (!mdOutput) return;
    const baseName = extractState?.fileName.replace(/\.[^.]+$/, "") ?? "schema";
    const fileName = `${baseName}.md`;
    const blob = new Blob([mdOutput], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = fileName;
    a.click();
    URL.revokeObjectURL(url);
    setDownloadSuccess(t.dc_download_success(fileName));
    setTimeout(() => setDownloadSuccess(null), 4000);
  }, [mdOutput, extractState, t]);

  const downloadRawMd = useCallback(() => {
    if (!extractState) return;
    const baseName = extractState.fileName.replace(/\.[^.]+$/, "");
    const fileName = `${baseName}.md`;
    const blob = new Blob([extractState.rawText], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = fileName;
    a.click();
    URL.revokeObjectURL(url);
    setDownloadSuccess(t.dc_download_success(fileName));
    setTimeout(() => setDownloadSuccess(null), 4000);
  }, [extractState, t]);

  return (
    <div className="doc-converter">
      <div className="doc-converter__header">
        <h2>📄 {t.dc_title}</h2>
        <p>{t.dc_subtitle}</p>
      </div>

      <div
        className="doc-converter__dropzone"
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDrop}
        onClick={handleDropzoneClick}
      >
        {extracting ? (
          <span>{t.dc_extracting}</span>
        ) : (
          <>
            <span className="doc-converter__dropzone-icon">📂</span>
            <span>{t.dc_dropzone_hint}</span>
            <span className="doc-converter__dropzone-hint">{t.dc_dropzone_formats}</span>
          </>
        )}
      </div>

      {error && (
        <div className="doc-converter__error" style={error.includes("\n") ? { whiteSpace: "pre-line" } : undefined}>
          {renderErrorWithLinks(error)}
        </div>
      )}

      {downloadSuccess && (
        <div className="doc-converter__success">{downloadSuccess}</div>
      )}

      <div className="doc-converter__toolbar">
        <span className="doc-converter__toolbar-label">{t.dc_model_label}</span>
        <select
          value={selectedProviderId}
          onChange={(e) => setSelectedProviderId(e.target.value)}
          className="doc-converter__select"
        >
          {providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.display_name} ({p.model}){p.is_default ? " ★" : ""}
            </option>
          ))}
          {providers.length === 0 && <option value="">{t.dc_model_none}</option>}
        </select>
      </div>

      {extractState && (
        <div className="doc-converter__raw-preview">
          <div className="doc-converter__raw-header">
            {t.dc_detected(extractState.fileName, "Markdown", extractState.rawText.length)}
          </div>
        </div>
      )}

      {extractState && !normalizing && (
        <div className="doc-converter__actions">
          <button
            className="doc-converter__btn doc-converter__btn--primary"
            onClick={normalizeWithAi}
            disabled={!selectedProviderId}
          >
            {t.dc_normalize_btn}
          </button>
          <button
            className="doc-converter__btn doc-converter__btn--secondary"
            onClick={downloadRawMd}
          >
            {t.dc_download_raw_btn}
          </button>
          {mdOutput && (
            <button
              className="doc-converter__btn doc-converter__btn--secondary"
              onClick={downloadMd}
            >
              {t.dc_download_btn}
            </button>
          )}
        </div>
      )}

      {normalizing && (
        <div className="doc-converter__actions">
          <div className="doc-converter__progress">
            <span>⟳</span>
            <span>
              {t.dc_normalizing}
              {normalizeProgress && t.dc_normalizing_step(normalizeProgress.step, normalizeProgress.total)}
            </span>
          </div>
          <button
            className="doc-converter__btn doc-converter__btn--secondary"
            onClick={() => { stoppedRef.current = true; }}
          >
            {t.dc_stop_btn}
          </button>
        </div>
      )}

      {mdOutput && (
        <div className="doc-converter__preview">
          <div className="doc-converter__preview-header">
            <span className="doc-converter__preview-label">
              {t.dc_preview_label(mdOutput.length)}
            </span>
            <button
              className="doc-converter__btn doc-converter__btn--secondary"
              style={{ fontSize: 11, padding: "2px 8px" }}
              onClick={downloadMd}
            >
              {t.dc_download_btn}
            </button>
          </div>
          <pre className="doc-converter__preview-box">{mdOutput}</pre>
        </div>
      )}
    </div>
  );
}
