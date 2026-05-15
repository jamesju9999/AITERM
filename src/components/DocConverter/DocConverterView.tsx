// src/components/DocConverter/DocConverterView.tsx
import { useState, useRef, useCallback, useEffect } from "react";
import * as XLSX from "xlsx";
import mammoth from "mammoth";
import * as pdfjsLib from "pdfjs-dist";
import { listen } from "@tauri-apps/api/event";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { aiChat, formatAiError } from "../../ipc/ai";
import { readFileAsArrayBuffer } from "../../ipc/fs";
import { useLocale } from "../../contexts/LocaleContext";
import "./DocConverterView.css";

// PDF.js worker
pdfjsLib.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

type Format = "excel" | "word" | "pdf";

interface ExtractState {
  format: Format;
  fileName: string;
  rawText: string;
}

function detectFormat(name: string): Format | null {
  const lower = name.toLowerCase();
  if (lower.endsWith(".xlsx") || lower.endsWith(".xls") || lower.endsWith(".csv")) return "excel";
  if (lower.endsWith(".docx")) return "word";
  if (lower.endsWith(".pdf")) return "pdf";
  return null;
}

async function extractExcel(buffer: ArrayBuffer): Promise<string> {
  const wb = XLSX.read(buffer, { type: "array" });
  const parts: string[] = [];
  for (const sheetName of wb.SheetNames) {
    const csv = XLSX.utils.sheet_to_csv(wb.Sheets[sheetName]);
    parts.push(`[Sheet: ${sheetName}]\n${csv}`);
  }
  return parts.join("\n\n");
}

async function extractWord(buffer: ArrayBuffer): Promise<string> {
  const result = await mammoth.extractRawText({ arrayBuffer: buffer });
  return result.value;
}

async function extractPdf(buffer: ArrayBuffer): Promise<string> {
  const pdf = await pdfjsLib.getDocument({ data: buffer }).promise;
  try {
    const pages: string[] = [];
    for (let i = 1; i <= pdf.numPages; i++) {
      const page = await pdf.getPage(i);
      const content = await page.getTextContent();
      const pageText = content.items
        // @ts-ignore
        .map((item) => (item.str ?? ""))
        .join(" ");
      pages.push(pageText);
    }
    return pages.join("\n");
  } finally {
    pdf.destroy();
  }
}

const CHUNK_SIZE = 3500; // chars per AI call

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
  const fileInputRef = useRef<HTMLInputElement>(null);
  const stoppedRef = useRef(false);

  const processFileData = useCallback(async (fileName: string, buffer: ArrayBuffer) => {
    stoppedRef.current = true;  // cancel any in-flight normalization
    setNormalizing(false);
    setNormalizeProgress(null);
    setError(null);
    setDownloadSuccess(null);
    setExtractState(null);
    setMdOutput("");
    const format = detectFormat(fileName);
    if (!format) {
      setError("不支援的格式。請使用 Excel (.xlsx), Word (.docx) 或 PDF (.pdf)");
      return;
    }
    setExtracting(true);
    try {
      let rawText = "";
      if (format === "excel") rawText = await extractExcel(buffer);
      else if (format === "word") rawText = await extractWord(buffer);
      else rawText = await extractPdf(buffer);

      setExtractState({ format, fileName, rawText });
    } catch (e) {
      setError(`提取失敗：${String(e)}`);
    } finally {
      setExtracting(false);
    }
  }, []);

  const processFile = useCallback(async (file: File) => {
    const buffer = await file.arrayBuffer();
    processFileData(file.name, buffer);
  }, [processFileData]);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
  }, []);

  // Listen for OS-level file drops (Tauri intercepts these before the web DOM sees them)
  useEffect(() => {
    type DragDropPayload = { paths: string[]; position?: unknown };
    const unlisten = listen<DragDropPayload>("tauri://drag-drop", async (event) => {
      const paths = event.payload?.paths;
      if (!paths?.length) return;
      const path = paths[0];
      const fileName = path.split(/[\\/]/).pop() ?? path;
      try {
        const buffer = await readFileAsArrayBuffer(path);
        processFileData(fileName, buffer);
      } catch (e) {
        setError(`讀取失敗：${String(e)}`);
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [processFileData]);

  const handleFileInput = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) processFile(file);
    e.target.value = "";
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    const file = e.dataTransfer.files[0];
    if (file) processFile(file);
  };

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
        const result = await aiChat(
          chunk,
          NORMALIZATION_SYSTEM_PROMPT,
          [],
          selectedProviderId || undefined,
        );
        parts.push(result.trim());
      } catch (e) {
        const aiErr = typeof e === "object" && e !== null && "kind" in e
          ? formatAiError(e as import("../../ipc/ai").AiError)
          : String(e);
        setError(`AI 正規化失敗（步驟 ${i + 1}）：${aiErr}`);
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
        onClick={() => fileInputRef.current?.click()}
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
        <input
          ref={fileInputRef}
          type="file"
          accept=".xlsx,.xls,.csv,.docx,.pdf"
          style={{ display: "none" }}
          onChange={handleFileInput}
        />
      </div>

      {error && (
        <div className="doc-converter__error">{error}</div>
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
            {t.dc_detected(extractState.fileName, extractState.format.toUpperCase(), extractState.rawText.length)}
          </div>
        </div>
      )}

      {/* AI normalization actions */}
      {extractState && !normalizing && (
        <div className="doc-converter__actions">
          <button
            className="doc-converter__btn doc-converter__btn--primary"
            onClick={normalizeWithAi}
            disabled={!selectedProviderId}
          >
            {t.dc_normalize_btn}
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

      {/* MD preview */}
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
