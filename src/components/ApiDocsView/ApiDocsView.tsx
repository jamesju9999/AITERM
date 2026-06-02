// src/components/ApiDocsView/ApiDocsView.tsx
import { useState, useEffect, useRef, useCallback } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import {
  apiDocsDetect,
  apiDocsFetchTree,
  apiDocsExtract,
  apiDocsLogin,
  apiDocsLogout,
  apiDocsAuthStatus,
  onApiDocsDetected,
  onApiDocsProgress,
  onApiDocsLog,
  onApiDocsDone,
  DEFAULT_KEEP_OPTIONS,
} from "../../ipc/apiDocs";
import { pickFolder } from "../../ipc/vcs";
import type {
  DocNode,
  KeepOptions,
  AuthStatus,
  ApiDocsLogEvent,
} from "../../ipc/apiDocs";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { readFile, writeTextFile } from "../../ipc/fs";
import { aiChat, formatAiError, type AiError } from "../../ipc/ai";
import { DocTree } from "./DocTree";
import { ExtractionSettings } from "./ExtractionSettings";
import { ExtractionLog } from "./ExtractionLog";
import "./ApiDocsView.css";

interface Props {
  isActive: boolean;
}

function extractDomain(url: string): string {
  return url
    .replace(/^https?:\/\//, "")
    .split("/")[0];
}

export function ApiDocsView({ isActive }: Props) {
  const { t } = useLocale();

  // URL input
  const [url, setUrl] = useState("");
  const [platform, setPlatform] = useState<string | null>(null);
  const [treeLoading, setTreeLoading] = useState(false);
  const [tree, setTree] = useState<DocNode[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");

  // Settings — persist output dir across sessions
  const [outputDir, setOutputDir] = useState(
    () => localStorage.getItem("api-docs-output-dir") ?? "~/api-docs"
  );
  const [merge, setMerge] = useState(true);
  const [keep, setKeep] = useState<KeepOptions>(DEFAULT_KEEP_OPTIONS);

  // Auth
  const [auth, setAuth] = useState<AuthStatus>({ logged_in: false, account: "" });
  const domain = extractDomain(url);

  // Extraction state
  const [extracting, setExtracting] = useState(false);
  const [progress, setProgress] = useState({ current: 0, total: 0 });
  const [logs, setLogs] = useState<ApiDocsLogEvent[]>([]);
  const [outputFiles, setOutputFiles] = useState<string[]>([]);

  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [translateToZh, setTranslateToZh] = useState(true);

  const mountedRef = useRef(true);
  const outputFilesRef = useRef<string[]>([]);
  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Subscribe to Tauri events
  useEffect(() => {
    let unlisteners: (() => void)[] = [];

    Promise.all([
      onApiDocsDetected((e) => {
        if (mountedRef.current) setPlatform(e.platform);
      }),
      onApiDocsProgress((e) => {
        if (mountedRef.current) setProgress({ current: e.current, total: e.total });
      }),
      onApiDocsLog((e) => {
        if (mountedRef.current) setLogs((prev) => [...prev, e]);
      }),
      onApiDocsDone((e) => {
        if (mountedRef.current) {
          setOutputFiles(e.files);
          outputFilesRef.current = e.files;
        }
      }),
    ]).then((fns) => {
      unlisteners = fns;
    });

    return () => unlisteners.forEach((fn) => fn());
  }, []);

  // Load providers on mount
  useEffect(() => {
    listProviders().then((list) => {
      if (!mountedRef.current) return;
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(() => {});
  }, []);

  // Refresh auth status when domain changes
  useEffect(() => {
    if (!domain) return;
    apiDocsAuthStatus(domain)
      .then((status) => { if (mountedRef.current) setAuth(status); })
      .catch(() => {});
  }, [domain]);

  const handleLoadTree = useCallback(async () => {
    if (!url.trim()) return;
    setTreeLoading(true);
    setPlatform(null);
    setTree([]);
    setSelected(new Set());
    setLogs([]);
    setOutputFiles([]);
    try {
      await apiDocsDetect(url);
      const nodes = await apiDocsFetchTree(url);
      if (mountedRef.current) setTree(nodes);
    } catch (err) {
      if (mountedRef.current) {
        setLogs([{ level: "error", message: String(err) }]);
      }
    } finally {
      if (mountedRef.current) setTreeLoading(false);
    }
  }, [url]);

  const handleOutputDirChange = useCallback((v: string) => {
    setOutputDir(v);
    localStorage.setItem("api-docs-output-dir", v);
  }, []);

  const handlePickFolder = useCallback(async () => {
    const folder = await pickFolder();
    if (folder) {
      setOutputDir(folder);
      localStorage.setItem("api-docs-output-dir", folder);
    }
  }, []);

  const handleSelectAll = useCallback(() => {
    const collect = (nodes: DocNode[]): string[] =>
      nodes.flatMap((n) => n.items.length ? collect(n.items) : [n.href]);
    setSelected(new Set(collect(tree)));
  }, [tree]);

  const handleDeselectAll = useCallback(() => setSelected(new Set()), []);

  const handleLogin = useCallback(async () => {
    try {
      await apiDocsLogin(url);
      const status = await apiDocsAuthStatus(domain);
      if (mountedRef.current) setAuth(status);
    } catch (err) {
      if (mountedRef.current) {
        setLogs((prev) => [...prev, { level: "error", message: String(err) }]);
      }
    }
  }, [url, domain]);

  const handleLogout = useCallback(async () => {
    await apiDocsLogout(domain);
    if (mountedRef.current) setAuth({ logged_in: false, account: "" });
  }, [domain]);

  const runExtract = useCallback(async () => {
    await apiDocsExtract({
      url,
      pages: Array.from(selected),
      output_dir: outputDir,
      merge,
      keep,
      cookies: "",
    });
  }, [url, selected, outputDir, merge, keep]);

  const startExtraction = useCallback(async () => {
    setExtracting(true);
    setLogs([]);
    setOutputFiles([]);
    setProgress({ current: 0, total: selected.size });
    try {
      await runExtract();
    } catch (err) {
      if (mountedRef.current) {
        setLogs((prev) => [...prev, { level: "error", message: String(err) }]);
      }
    } finally {
      if (mountedRef.current) setExtracting(false);
    }
  }, [runExtract, selected.size]);

  const AI_SYSTEM_PROMPT = translateToZh
    ? `你是 API 技術文件整理工具。
將以下 API 文件頁面的原始文字整理成清晰的繁體中文 Markdown 格式。
- 將內容翻譯成繁體中文，技術術語可保留英文（如 endpoint、token、OAuth 等）
- 保留所有技術細節：URL、參數名稱、代碼範例、環境說明、Token 相關資訊
- 用適當的 Markdown 標題（##、###）組織內容
- 用 Markdown 表格整理環境/Scope 對應關係
- 移除重複的導航連結和版權文字
- 保持精簡，不要加入原文沒有的資訊`
    : `You are an API documentation formatting tool.
Reformat the following raw API documentation page into clean Markdown.
- Preserve all technical details: URLs, parameter names, code examples, environment info, token-related content
- Organize with appropriate Markdown headings (##, ###)
- Use Markdown tables for environment/scope mappings
- Remove duplicate navigation links and copyright text
- Be concise, do not add information not present in the original`;

  // Split a large Markdown section into chunks of ≤ maxChars.
  // Splits at ## and ### heading boundaries so each chunk stays self-contained.
  function splitIntoChunks(text: string, maxChars: number): string[] {
    const parts = text.split(/(?=\n#{2,3} )/);
    const chunks: string[] = [];
    let current = "";
    for (const part of parts) {
      if (current.length + part.length > maxChars && current.length > 0) {
        chunks.push(current);
        current = part;
      } else {
        current += part;
      }
    }
    if (current.length > 0) chunks.push(current);
    return chunks;
  }

  const startExtractionAi = useCallback(async () => {
    if (!selectedProviderId) return;
    setExtracting(true);
    setLogs([]);
    setOutputFiles([]);
    setProgress({ current: 0, total: selected.size });
    let files: string[] = [];
    try {
      await runExtract();
      // Yield one microtask tick so the done event handler can run
      await new Promise<void>((r) => setTimeout(r, 50));
      files = outputFilesRef.current;
    } catch (err) {
      if (mountedRef.current) {
        setLogs((prev) => [...prev, { level: "error", message: String(err) }]);
      }
    }

    // Post-process: AI-enhance sections that need it.
    // When translateToZh is true, ALL sections are processed (for translation).
    // When false, only sections with the needs-AI marker are processed.
    const AI_MARKER = "> ⚠ AI processing required";
    const SECTION_SEP = "\n\n---\n\n";

    for (const filePath of files) {
      if (!mountedRef.current) break;
      try {
        const { content } = await readFile(filePath);
        const hasAnyMarker = content.includes(AI_MARKER);
        if (!translateToZh && !hasAnyMarker) continue;

        // Split into sections (merged file has multiple; single file has one)
        const sections = content.split(SECTION_SEP);
        let changed = false;
        const newSections: string[] = [];

        for (const section of sections) {
          const hasMarker = section.includes(AI_MARKER);

          // Skip if no translation requested and no marker
          if (!translateToZh && !hasMarker) {
            newSections.push(section);
            continue;
          }

          const titleLine = section.split("\n")[0] ?? "";
          let textToProcess: string;

          if (hasMarker) {
            // Extract raw text after the marker
            const markerIdx = section.indexOf(AI_MARKER);
            textToProcess = section.slice(markerIdx + AI_MARKER.length).trim();
            if (!textToProcess) {
              newSections.push(section);
              continue;
            }
          } else {
            textToProcess = section;
          }

          const CHUNK_MAX = 3_000; // safe limit for local models (~750 tokens)
          const chunks = textToProcess.length > CHUNK_MAX
            ? splitIntoChunks(textToProcess, CHUNK_MAX)
            : [textToProcess];
          const totalChunks = chunks.length;

          const actionLabel = translateToZh ? "翻譯中" : "AI 增強中";
          setLogs((prev) => [...prev, {
            level: "info",
            message: `🤖 ${actionLabel}：${titleLine.replace(/^#+ /, "")}${totalChunks > 1 ? ` (分 ${totalChunks} 塊)` : ""}`,
          }]);

          try {
            const translatedChunks: string[] = [];
            for (let ci = 0; ci < chunks.length; ci++) {
              if (!mountedRef.current) break;
              // Brief pause between chunks to let local model server recover
              if (ci > 0) await new Promise<void>((r) => setTimeout(r, 300));
              const result = await aiChat(chunks[ci], AI_SYSTEM_PROMPT, [], selectedProviderId);
              translatedChunks.push(result.trim());
            }

            const enhanced = translatedChunks.join("\n\n");
            newSections.push(hasMarker
              ? `${titleLine}\n\n${enhanced}`
              : enhanced
            );
            changed = true;

            const doneLabel = translateToZh ? "翻譯完成" : "AI 增強完成";
            setLogs((prev) => [...prev, {
              level: "info",
              message: `✓ ${doneLabel}：${titleLine.replace(/^#+ /, "")}`,
            }]);
          } catch (sectionErr) {
            if (!mountedRef.current) break;
            const msg = sectionErr instanceof Error
              ? sectionErr.message
              : formatAiError(sectionErr as AiError);
            setLogs((prev) => [...prev, {
              level: "error",
              message: `✗ 失敗（保留原文）：${titleLine.replace(/^#+ /, "")} — ${msg}`,
            }]);
            newSections.push(section);
          }
        }

        if (changed) {
          await writeTextFile(filePath, newSections.join(SECTION_SEP) + "\n");
        }
      } catch (err) {
        if (mountedRef.current) {
          setLogs((prev) => [...prev, {
            level: "error",
            message: `AI 增強失敗：${err instanceof Error ? err.message : formatAiError(err as AiError)}`,
          }]);
        }
      }
    }

    if (mountedRef.current) setExtracting(false);
  }, [runExtract, selected.size, selectedProviderId, outputDir]);

  if (!isActive) return null;

  return (
    <div className="api-docs-view">
      {/* URL bar */}
      <div className="api-docs-view__url-bar">
        <input
          className="api-docs-view__url-input"
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder={t.api_docs_url_placeholder}
          onKeyDown={(e) => e.key === "Enter" && handleLoadTree()}
        />
        <button
          className="api-docs-view__load-btn"
          onClick={handleLoadTree}
          disabled={treeLoading || !url.trim()}
        >
          {treeLoading ? t.api_docs_loading : t.api_docs_load_tree}
        </button>
        {platform && (
          <span className="api-docs-view__platform-badge">
            {t.api_docs_platform_label}: {platform}
          </span>
        )}
      </div>

      {/* Main layout: left tree + right settings */}
      <div className="api-docs-view__body">
        {/* Left: tree */}
        <div className="api-docs-view__tree-panel">
          {tree.length > 0 && (
            <>
              <div className="api-docs-view__tree-toolbar">
                <input
                  className="api-docs-view__filter-input"
                  type="text"
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  placeholder={t.api_docs_filter_placeholder}
                />
                <button className="api-docs-view__toolbar-btn" onClick={handleSelectAll}>
                  {t.api_docs_select_all}
                </button>
                <button className="api-docs-view__toolbar-btn" onClick={handleDeselectAll}>
                  {t.api_docs_deselect_all}
                </button>
                <span className="api-docs-view__selected-count">
                  {t.api_docs_pages_selected(selected.size)}
                </span>
              </div>
              <div className="api-docs-view__tree-scroll">
                <DocTree
                  nodes={tree}
                  selected={selected}
                  onChange={setSelected}
                  filter={filter}
                />
              </div>
            </>
          )}
          {tree.length === 0 && !treeLoading && (
            <div className="api-docs-view__empty">
              {url ? t.api_docs_load_tree : t.api_docs_url_placeholder}
            </div>
          )}
          {(logs.length > 0 || extracting) && (
            <ExtractionLog
              current={progress.current}
              total={progress.total}
              logs={logs}
              outputFiles={outputFiles}
            />
          )}
        </div>

        {/* Right: settings */}
        <div className="api-docs-view__settings-panel">
          <ExtractionSettings
            outputDir={outputDir}
            onOutputDirChange={handleOutputDirChange}
            onPickFolder={handlePickFolder}
            merge={merge}
            onMergeChange={setMerge}
            keep={keep}
            onKeepChange={setKeep}
            auth={auth}
            domain={domain}
            onLogin={handleLogin}
            onLogout={handleLogout}
            extracting={extracting}
            selectedCount={selected.size}
            providers={providers}
            selectedProviderId={selectedProviderId}
            onProviderChange={setSelectedProviderId}
            translateToZh={translateToZh}
            onTranslateToZhChange={setTranslateToZh}
            onExtractRaw={startExtraction}
            onExtractAi={startExtractionAi}
          />
        </div>
      </div>
    </div>
  );
}
