// src/components/ApiDocsView/ApiDocsView.tsx
import { useState, useEffect, useRef, useCallback } from "react";
import { useNavigate } from "react-router-dom";
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
import { usePythonEnvGate } from "../PythonEnv/usePythonEnvGate";
import { PythonEnvGate } from "../PythonEnv/PythonEnvGate";
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
  const navigate = useNavigate();

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

  const pythonEnv = usePythonEnvGate();

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
    const ready = await pythonEnv.ensureProfile("api_docs");
    if (!ready) {
      if (mountedRef.current) setTreeLoading(false);
      return;
    }
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
  }, [url, pythonEnv.ensureProfile]);

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

  // Retry aiChat on connection errors (local model server crash/restart).
  // Uses exponential backoff: 3s → 6s → 12s between attempts.
  async function aiChatWithRetry(
    text: string,
    prompt: string,
    providerId: string,
    maxRetries = 3,
  ): Promise<string> {
    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      try {
        const result = await aiChat(
          [{ role: "system" as const, content: prompt }, { role: "user" as const, content: text }],
          "apidocs",
          providerId,
        );
        return result.content ?? "";
      } catch (err) {
        const isConnectionErr =
          typeof err === "object" && err !== null &&
          (err as AiError).kind === "network" &&
          ((err as { kind: string; message?: string }).message ?? "").includes("error sending request");

        if (isConnectionErr && attempt < maxRetries) {
          const delay = 3000 * Math.pow(2, attempt); // 3s, 6s, 12s
          setLogs((prev) => [...prev, {
            level: "info",
            message: `⏳ 本地模型無回應，${delay / 1000}s 後重試 (${attempt + 1}/${maxRetries})…`,
          }]);
          await new Promise<void>((r) => setTimeout(r, delay));
          continue;
        }
        throw err;
      }
    }
    throw new Error("max retries exceeded");
  }

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
    const CHUNK_MAX = 3_000; // safe limit for local models (~750 tokens)

    // Pre-read all files and count translatable *chunks* (not sections) so the
    // progress bar advances once per AI call, even when a section is split into
    // many chunks.
    type FileEntry = { filePath: string; content: string; sections: string[] };
    const fileEntries: FileEntry[] = [];
    let aiTotal = 0;
    for (const filePath of files) {
      if (!mountedRef.current) break;
      try {
        const { content } = await readFile(filePath);
        const hasAnyMarker = content.includes(AI_MARKER);
        if (!translateToZh && !hasAnyMarker) continue;
        const sections = content.split(SECTION_SEP);
        fileEntries.push({ filePath, content, sections });
        for (const section of sections) {
          if (!translateToZh && !section.includes(AI_MARKER)) continue;
          const hasMarker = section.includes(AI_MARKER);
          let text = section;
          if (hasMarker) {
            const idx = section.indexOf(AI_MARKER);
            text = section.slice(idx + AI_MARKER.length).trim();
            if (!text) { aiTotal++; continue; } // empty marker = 1 no-op unit
          }
          aiTotal += text.length > CHUNK_MAX ? splitIntoChunks(text, CHUNK_MAX).length : 1;
        }
      } catch {
        // skip unreadable files
      }
    }

    let aiCurrent = 0;
    if (aiTotal > 0) setProgress({ current: 0, total: aiTotal });

    for (const { filePath, sections } of fileEntries) {
      if (!mountedRef.current) break;
      try {
        let changed = false;
        const newSections: string[] = [];

        for (let sectionIdx = 0; sectionIdx < sections.length; sectionIdx++) {
          const section = sections[sectionIdx];
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
              aiCurrent++;
              setProgress({ current: aiCurrent, total: aiTotal });
              newSections.push(section);
              continue;
            }
          } else {
            textToProcess = section;
          }

          const chunks = textToProcess.length > CHUNK_MAX
            ? splitIntoChunks(textToProcess, CHUNK_MAX)
            : [textToProcess];
          const totalChunks = chunks.length;

          const actionLabel = translateToZh ? "翻譯中" : "AI 增強中";
          setLogs((prev) => [...prev, {
            level: "info",
            message: `🤖 ${actionLabel}：${titleLine.replace(/^#+ /, "")}${totalChunks > 1 ? ` (分 ${totalChunks} 塊)` : ""}`,
          }]);

          // Pause between sections so local models (OMLX) can unload before next request
          if (sectionIdx > 0) await new Promise<void>((r) => setTimeout(r, 1000));

          let chunksDone = 0;
          try {
            const translatedChunks: string[] = [];
            for (let ci = 0; ci < chunks.length; ci++) {
              if (!mountedRef.current) break;
              // Brief pause between chunks to let local model server recover
              if (ci > 0) await new Promise<void>((r) => setTimeout(r, 300));
              const result = await aiChatWithRetry(chunks[ci], AI_SYSTEM_PROMPT, selectedProviderId);
              translatedChunks.push(result.trim());
              chunksDone++;
              aiCurrent++;
              setProgress({ current: aiCurrent, total: aiTotal });
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
            // Advance past any remaining chunks for this section that weren't processed.
            aiCurrent += chunks.length - chunksDone;
            setProgress({ current: aiCurrent, total: aiTotal });
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

      <PythonEnvGate
        state={pythonEnv.state}
        lines={pythonEnv.lines}
        error={pythonEnv.error}
        onInstall={() => pythonEnv.ensureProfile("api_docs")}
        onRecheck={() => pythonEnv.ensureProfile("api_docs")}
        onPickInterpreter={() => navigate("/settings", { state: { tab: "general" } })}
      />

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
