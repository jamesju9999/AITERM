import { useState, useEffect, useCallback } from "react";
import { listDirectory, getSessionCwd, type DirEntry } from "../../ipc/fs";
import "./FileExplorer.css";

interface FileExplorerProps {
  sessionId: string;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function getFileIcon(entry: DirEntry): string {
  if (entry.is_dir) return "📁";
  const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
  const icons: Record<string, string> = {
    ts: "🔷", tsx: "🔷", js: "🟡", jsx: "🟡",
    json: "📋", md: "📝", rs: "🦀", toml: "⚙️",
    css: "🎨", html: "🌐", png: "🖼️", jpg: "🖼️",
    jpeg: "🖼️", gif: "🖼️", svg: "🎨", sh: "⚡",
    py: "🐍", go: "🔵", yaml: "⚙️", yml: "⚙️",
    txt: "📄", pdf: "📕", zip: "🗜️", tar: "🗜️",
    gz: "🗜️", lock: "🔒",
  };
  return icons[ext] ?? "📄";
}

export function FileExplorer({ sessionId }: FileExplorerProps) {
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [cwd, setCwd] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [subEntries, setSubEntries] = useState<Record<string, DirEntry[]>>({});
  const [showDotfiles, setShowDotfiles] = useState(false);

  const loadDir = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const result = await listDirectory(sessionId, path);
      const filtered = showDotfiles ? result : result.filter(e => !e.name.startsWith("."));
      setEntries(filtered);
      setCwd(path || (await getSessionCwd(sessionId)) || "");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId, showDotfiles]);

  useEffect(() => {
    loadDir("");
  }, [loadDir]);

  const handleToggleDir = async (entry: DirEntry) => {
    if (!entry.is_dir) return;
    const key = entry.path;
    if (expanded.has(key)) {
      const next = new Set(expanded);
      next.delete(key);
      setExpanded(next);
    } else {
      if (!subEntries[key]) {
        try {
          const children = await listDirectory(sessionId, key);
          setSubEntries(prev => ({ ...prev, [key]: children }));
        } catch {
          // silently ignore
        }
      }
      setExpanded(prev => new Set([...prev, key]));
    }
  };

  const goUp = () => {
    const parent = cwd.split("/").slice(0, -1).join("/") || "/";
    loadDir(parent);
    setExpanded(new Set());
    setSubEntries({});
  };

  const renderEntries = (list: DirEntry[], depth = 0) => (
    <>
      {list.map((entry) => (
        <div key={entry.path}>
          <div
            className={`fe-row ${entry.is_dir ? "fe-row--dir" : ""}`}
            style={{ paddingLeft: `${12 + depth * 16}px` }}
            onClick={() => entry.is_dir ? handleToggleDir(entry) : undefined}
          >
            <span className="fe-arrow">
              {entry.is_dir ? (expanded.has(entry.path) ? "▼" : "▶") : " "}
            </span>
            <span className="fe-icon">{getFileIcon(entry)}</span>
            <span className="fe-name">{entry.name}</span>
            {entry.size != null && (
              <span className="fe-size">{formatSize(entry.size)}</span>
            )}
          </div>
          {entry.is_dir && expanded.has(entry.path) && subEntries[entry.path] && (
            renderEntries(subEntries[entry.path], depth + 1)
          )}
        </div>
      ))}
    </>
  );

  const cwdParts = cwd.split("/").filter(Boolean);

  return (
    <div className="file-explorer">
      {/* Toolbar */}
      <div className="fe-toolbar">
        <button className="fe-btn" onClick={goUp} title="上一層" disabled={!cwd || cwd === "/"}>
          ↑
        </button>
        <button className="fe-btn" onClick={() => loadDir(cwd)} title="重新整理">
          ↻
        </button>
        <div className="fe-breadcrumb">
          <span
            className="fe-breadcrumb-item"
            onClick={() => loadDir("/")}
          >/</span>
          {cwdParts.map((part, i) => {
            const path = "/" + cwdParts.slice(0, i + 1).join("/");
            return (
              <span key={path}>
                <span className="fe-breadcrumb-sep">/</span>
                <span
                  className="fe-breadcrumb-item"
                  onClick={() => loadDir(path)}
                >
                  {part}
                </span>
              </span>
            );
          })}
        </div>
        <button
          className={`fe-btn fe-btn--dot ${showDotfiles ? "fe-btn--active" : ""}`}
          onClick={() => setShowDotfiles(p => !p)}
          title="顯示/隱藏隱藏檔案"
        >
          .
        </button>
      </div>

      {/* Body */}
      <div className="fe-body">
        {loading && <div className="fe-status">載入中…</div>}
        {error && <div className="fe-status fe-status--error">{error}</div>}
        {!loading && !error && entries.length === 0 && (
          <div className="fe-status">（空目錄）</div>
        )}
        {!loading && !error && renderEntries(entries)}
      </div>
    </div>
  );
}
