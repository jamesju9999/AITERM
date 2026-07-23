import { useState, useEffect, useCallback, useRef } from "react";
import { listDirectory, getSessionCwd, type DirEntry } from "../../ipc/fs";
import { FileViewer } from "./FileViewer";
import { useLocale } from "../../contexts/LocaleContext";
import "./FileExplorer.css";

interface FileExplorerProps {
  sessionId: string;
  /** Called with the currently-browsed directory when the user wants the
   * terminal session to `cd` there too. Omit to hide the "switch terminal
   * here" button (e.g. if the caller doesn't have a terminal to sync). */
  onSwitchTerminalHere?: (path: string) => void;
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

export function FileExplorer({ sessionId, onSwitchTerminalHere }: FileExplorerProps) {
  const { t } = useLocale();
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [cwd, setCwd] = useState<string>("");
  // The breadcrumb bar and "switch terminal here" both track this instead of
  // `cwd` directly: `cwd` is the root of the currently-listed `entries` (only
  // changes on a real navigation — breadcrumb click, "up", or a terminal-
  // driven change), but expanding a folder in the tree does NOT navigate
  // (entries stay rooted at `cwd`, the folder's children render inline) — so
  // without this, clicking deeper into the tree would leave the breadcrumb
  // and "cd" target stuck on the original root instead of following along.
  const [focusedPath, setFocusedPath] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [subEntries, setSubEntries] = useState<Record<string, DirEntry[]>>({});
  const [showDotfiles, setShowDotfiles] = useState(false);
  const [selectedFile, setSelectedFile] = useState<DirEntry | null>(null);
  // Tracks the last PTY CWD we observed — used only by the polling loop to
  // detect terminal-driven directory changes. Must NOT be updated on user-
  // initiated file-explorer navigation, otherwise the poll would revert the
  // user's manual browsing back to the terminal's CWD.
  const ptyCwdRef = useRef<string>("");

  const loadDir = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    try {
      const result = await listDirectory(sessionId, path);
      const filtered = showDotfiles ? result : result.filter(e => !e.name.startsWith("."));
      setEntries(filtered);
      const resolvedCwd = path || (await getSessionCwd(sessionId)) || "";
      setCwd(resolvedCwd);
      setFocusedPath(resolvedCwd);
    } catch (e) {
      setError(String(e));
      // On error, still try to reflect the current CWD so the breadcrumb isn't blank.
      if (!path) {
        try {
          const fallbackCwd = await getSessionCwd(sessionId);
          if (fallbackCwd) {
            setCwd(fallbackCwd);
            setFocusedPath(fallbackCwd);
          }
        } catch { /* ignore */ }
      }
    } finally {
      setLoading(false);
    }
  }, [sessionId, showDotfiles]);

  // Seed ptyCwdRef with the terminal's initial CWD on mount.
  useEffect(() => {
    getSessionCwd(sessionId)
      .then((c) => { if (c) ptyCwdRef.current = c; })
      .catch(() => {});
  }, [sessionId]);

  useEffect(() => {
    loadDir("");
  }, [loadDir]);

  // Poll terminal CWD and reload only when the PTY itself changed directories.
  // We compare against ptyCwdRef (last PTY-driven CWD) so that the user can
  // freely browse the file explorer without the poll reverting their navigation.
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const newCwd = await getSessionCwd(sessionId);
        if (newCwd && newCwd !== ptyCwdRef.current) {
          ptyCwdRef.current = newCwd;
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
    setFocusedPath(entry.path);
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
    const parts = normalized.split("/").filter(Boolean);
    parts.pop();
    // Windows drive root: ["C:"] after pop → [] but we already guard the button
    const parent = parts.length === 0
      ? "/"
      : parts.length === 1 && parts[0].endsWith(":")
        ? parts[0] + "/"
        : parts.join("/") || "/";
    loadDir(parent);
    setExpanded(new Set());
    setSubEntries({});
  };

  // True when we're already at a filesystem root (Unix "/" or Windows "C:/")
  const atRoot = !cwd || cwd === "/" || /^[A-Za-z]:\/?\s*$/.test(cwd.replace(/\\/g, "/"));

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

  const focusedParts = focusedPath.replace(/\\/g, "/").split("/").filter(Boolean);

  return (
    <div className="file-explorer">
      {/* Toolbar */}
      <div className="fe-toolbar">
        <button className="fe-btn aiterm-btn aiterm-btn--secondary" onClick={goUp} title={t.file_go_up} disabled={atRoot}>
          ↑
        </button>
        <button className="fe-btn aiterm-btn aiterm-btn--secondary" onClick={() => loadDir(cwd)} title={t.file_refresh}>
          ↻
        </button>
        <div className="fe-breadcrumb">
          <span className="fe-breadcrumb-item" onClick={() => {
            if (!focusedPath) { loadDir(""); return; }
            // On Windows the path starts with a drive letter (e.g. "C:/Users/…") —
            // navigate to the drive root instead of the invalid "/" path.
            const firstPart = focusedPath.replace(/\\/g, "/").split("/")[0];
            loadDir(firstPart.endsWith(":") ? firstPart + "/" : "/");
          }}>
            /
          </span>
          {focusedParts.map((part, i) => {
            const seg = focusedParts.slice(0, i + 1).join("/");
            // Windows drive root "C:" → "C:/"; Windows sub-path "C:/Users" → "C:/Users"; Unix → "/Users"
            const path = seg.endsWith(":")
              ? seg + "/"
              : /^[A-Za-z]:/.test(seg) ? seg : "/" + seg;
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
          className={`fe-btn fe-btn--dot aiterm-btn aiterm-btn--secondary ${showDotfiles ? "fe-btn--active" : ""}`}
          onClick={() => setShowDotfiles(p => !p)}
          title={t.file_toggle_dotfiles}
        >
          .
        </button>
        {onSwitchTerminalHere && (
          <button
            className="fe-btn aiterm-btn aiterm-btn--secondary"
            onClick={() => onSwitchTerminalHere(focusedPath)}
            title={t.file_switch_terminal_here}
            disabled={!focusedPath}
          >
            {t.file_switch_terminal_here_short}
          </button>
        )}
      </div>

      {/* Split body */}
      <div className="fe-split">
        {/* Left: file tree */}
        <div className="fe-left">
          <div className="fe-body">
            {loading && <div className="fe-status">{t.file_loading}</div>}
            {error && <div className="fe-status fe-status--error">{error}</div>}
            {!loading && !error && entries.length === 0 && (
              <div className="fe-status">{t.file_empty_dir}</div>
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
