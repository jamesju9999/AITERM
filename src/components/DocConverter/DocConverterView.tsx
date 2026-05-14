// src/components/DocConverter/DocConverterView.tsx
import { useState, useRef, useCallback, useEffect } from "react";
import * as XLSX from "xlsx";
import mammoth from "mammoth";
import * as pdfjsLib from "pdfjs-dist";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
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

export function DocConverterView({ isActive: _isActive }: { isActive: boolean }) {
  const [extractState, setExtractState] = useState<ExtractState | null>(null);
  const [extracting, setExtracting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const fileInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
  }, []);

  const processFile = useCallback(async (file: File) => {
    setError(null);
    setExtractState(null);
    const format = detectFormat(file.name);
    if (!format) {
      setError("不支援的格式。請使用 Excel (.xlsx), Word (.docx) 或 PDF (.pdf)");
      return;
    }
    setExtracting(true);
    try {
      const buffer = await file.arrayBuffer();
      let rawText = "";
      if (format === "excel") rawText = await extractExcel(buffer);
      else if (format === "word") rawText = await extractWord(buffer);
      else rawText = await extractPdf(buffer);

      setExtractState({ format, fileName: file.name, rawText });
    } catch (e) {
      setError(`提取失敗：${String(e)}`);
    } finally {
      setExtracting(false);
    }
  }, []);

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

  return (
    <div className="doc-converter">
      <div className="doc-converter__header">
        <h2>📄 文件轉換器</h2>
        <p>將 Word / PDF / Excel 資料字典轉換成結構化 Markdown Schema 文件</p>
      </div>

      <div
        className="doc-converter__dropzone"
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDrop}
        onClick={() => fileInputRef.current?.click()}
      >
        {extracting ? (
          <span>⟳ 提取中...</span>
        ) : (
          <>
            <span className="doc-converter__dropzone-icon">📂</span>
            <span>拖放或點擊選擇檔案</span>
            <span className="doc-converter__dropzone-hint">支援 .xlsx .xls .csv .docx .pdf</span>
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

      <div className="doc-converter__toolbar">
        <span className="doc-converter__toolbar-label">AI 模型</span>
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
          {providers.length === 0 && <option value="">（未設定）</option>}
        </select>
      </div>

      {extractState && (
        <div className="doc-converter__raw-preview">
          <div className="doc-converter__raw-header">
            偵測到：{extractState.fileName}（{extractState.format.toUpperCase()}）
            · 提取 {extractState.rawText.length.toLocaleString()} 字元
          </div>
        </div>
      )}
    </div>
  );
}
