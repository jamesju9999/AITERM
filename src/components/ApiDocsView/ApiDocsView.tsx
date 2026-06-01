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
import type {
  DocNode,
  KeepOptions,
  AuthStatus,
  ApiDocsLogEvent,
} from "../../ipc/apiDocs";
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

  // Settings
  const [outputDir, setOutputDir] = useState("~/api-docs");
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

  // Placeholder — real provider detection would call list_providers()
  // For now, assume provider is available (disable AI button only when explicitly not configured)
  const [hasProvider] = useState(true);

  const mountedRef = useRef(true);
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
          setExtracting(false);
          setOutputFiles(e.files);
        }
      }),
    ]).then((fns) => {
      unlisteners = fns;
    });

    return () => unlisteners.forEach((fn) => fn());
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

  const startExtraction = useCallback(async () => {
    setExtracting(true);
    setLogs([]);
    setOutputFiles([]);
    setProgress({ current: 0, total: selected.size });
    try {
      await apiDocsExtract({
        url,
        pages: Array.from(selected),
        output_dir: outputDir,
        merge,
        keep,
        cookies: "",  // Rust layer reads from keyring
      });
    } catch (err) {
      if (mountedRef.current) {
        setExtracting(false);
        setLogs((prev) => [...prev, { level: "error", message: String(err) }]);
      }
    }
  }, [url, selected, outputDir, merge, keep]);

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
            onOutputDirChange={setOutputDir}
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
            hasProvider={hasProvider}
            onExtractRaw={startExtraction}
            onExtractAi={startExtraction}
          />
        </div>
      </div>
    </div>
  );
}
