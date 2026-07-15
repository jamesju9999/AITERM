import { useState, useEffect } from "react";
import { readFile } from "../../ipc/fs";
import type { DirEntry } from "../../ipc/fs";
import { useLocale } from "../../contexts/LocaleContext";
interface FileViewerProps {
  file: DirEntry | null;
}

type ViewState =
  | { kind: "empty" }
  | { kind: "loading" }
  | { kind: "ok"; content: string; truncated: boolean }
  | { kind: "binary" }
  | { kind: "error"; message: string };

export function FileViewer({ file }: FileViewerProps) {
  const { t } = useLocale();
  const [state, setState] = useState<ViewState>({ kind: "empty" });

  useEffect(() => {
    if (!file) {
      setState({ kind: "empty" });
      return;
    }
    setState({ kind: "loading" });
    readFile(file.path)
      .then(({ content, truncated }) =>
        setState({ kind: "ok", content, truncated })
      )
      .catch((err: unknown) => {
        const msg = err === "binary" || String(err) === "binary"
          ? "binary"
          : err instanceof Error ? err.message : String(err);
        if (msg === "binary") {
          setState({ kind: "binary" });
        } else {
          setState({ kind: "error", message: msg });
        }
      });
  }, [file?.path]);

  if (state.kind === "empty") {
    return (
      <div className="fv-empty">
        {t.file_viewer_empty}
      </div>
    );
  }

  return (
    <div className="fv-root">
      {file && (
        <div className="fv-header" title={file.path}>
          {file.name}
        </div>
      )}

      {state.kind === "loading" && (
        <div className="fv-status">{t.file_loading}</div>
      )}

      {state.kind === "binary" && (
        <div className="fv-status fv-status--muted">
          {t.file_viewer_binary}
        </div>
      )}

      {state.kind === "error" && (
        <div className="fv-status fv-status--error">{state.message}</div>
      )}

      {state.kind === "ok" && (
        <>
          {state.truncated && (
            <div className="fv-banner">{t.file_viewer_too_large}</div>
          )}
          <div className="fv-content">
            <pre className="fv-pre">
              {state.content.split("\n").map((line, i) => (
                <div key={i} className="fv-line">
                  <span className="fv-lineno">{i + 1}</span>
                  <span className="fv-linetext">{line}</span>
                </div>
              ))}
            </pre>
          </div>
        </>
      )}
    </div>
  );
}
