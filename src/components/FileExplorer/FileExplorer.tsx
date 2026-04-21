import { useState, useEffect, useCallback, useRef } from "react";
import { listDirectory, getSessionCwd, type DirEntry } from "../../ipc/fs";
import { FileViewer } from "./FileViewer";
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
  const [selectedFile, setSelectedFile] = useState<DirEntry | null>(null);
  const cwdRef = useRef<string>("");

  const loadDir = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const result = await listDirectory(sessionId, path);
      const filtered = showDotfiles ? result : result.filter(e => !e.name.startsWith("."));
      setEntries(filtered);
      const resolvedCwd = path || (await getSessionCwd(sessionId)) || "";
      setCwd(resolvedCwd);
      cwdRef.current = resolvedCwd;
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId, showDotfiles]);

  useEffect(() => {
    loadDir("");
  }, [loadDir]);

  // Poll terminal CWD and reload the file list when it changes.
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const newCwd = await getSessionCwd(sessionId);
        if (newCwd && newCwd !== cwdRef.current) {
          loadDir(newCwd);
          setExpanded(new Set());
          setSubEntries({});
        }
      } catch { /* ignore */ }
    }, 1500);
    return () => clearInterval(interval);
  }, [sessionId, loadDir]);

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
    // Normalize to forward slashes (already done by Rust, but defensive)
    const normalized = cwd.replace(/\\/g, "/").replace(/\/$/, "");
    const parts = normalized.split("/");
    parts.pop();
    // Windows drive root: ["C:"] → "C:/" ; Unix root: [] → "/"
    const parent = parts.length === 0
      ? "/"
      : parts.length === 1 && parts[0].endsWith(":")
        ? parts[0] + "/"
        : parts.join("/") || "/";
    loadDir(parent);
    setExpanded(new Set());
    setSubEntries({});
  };

  const renderEntries = (list: DirEntry[], depth = 0) => (
    <>
      {list.map((entry) => (
        <div key={entry.path}>
          <div
            className={`fe-row ${entry.is_dir ? "fe-row--dir" : "fe-row--file"} ${selectedFile?.path === entry.path ? "fe-row--selected" : ""}`}
            style={{ paddingLeft: `${12 + depth * 16}px` }}
            onClick={() => {
              if (entry.is_dir) {
                handleToggleDir(entry);
              } else {
                setSelectedFile(entry);
              }
            }}
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

  const cwdParts = cwd.replace(/\\/g, "/").split("/").filter(Boolean);

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
          <span className="fe-breadcrumb-item" onClick={() => loadDir("/")}>
            /
          </span>
          {cwdParts.map((part, i) => {
            const seg = cwdParts.slice(0, i + 1).join("/");
            // Windows drive root (e.g. "C:") needs trailing slash to be a valid path
            const path = seg.endsWith(":") ? seg + "/" : "/" + seg;
            return (
              <span key={path}>
                <span className="fe-breadcrumb-sep">/</span>
                <span className="fe-breadcrumb-item" onClick={() => loadDir(path)}>
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

      {/* Split body */}
      <div className="fe-split">
        {/* Left: file tree */}
        <div className="fe-left">
          <div className="fe-body">
            {loading && <div className="fe-status">載入中…</div>}
            {error && <div className="fe-status fe-status--error">{error}</div>}
            {!loading && !error && entries.length === 0 && (
              <div className="fe-status">（空目錄）</div>
            )}
            {!loading && !error && renderEntries(entries)}
          </div>
        </div>

        {/* Right: file viewer */}
        <div className="fe-right">
          <FileViewer file={selectedFile} />
        </div>
      </div>
    </div>
  );
}
