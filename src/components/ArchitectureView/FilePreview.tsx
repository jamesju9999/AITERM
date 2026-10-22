import { useEffect, useRef, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { archOpenPath, archReadFile, type FilePreview as Preview } from "../../ipc/architecture";

const MAX_LINES = 3000;

interface Props {
  root: string;
  /** 專案內的相對路徑（以 / 分隔）。 */
  path: string;
  /** 要標出並捲到可視範圍的行號（從 1 起算）。 */
  line?: number;
  onClose: () => void;
}

type State =
  | { key: string; kind: "ok"; data: Preview }
  | { key: string; kind: "error"; msg: string };

const errMsg = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

export function FilePreview({ root, path, line, onClose }: Props) {
  const { t } = useLocale();
  const key = `${root}\n${path}`;
  const [state, setState] = useState<State | null>(null);
  const [copied, setCopied] = useState(false);
  const hitRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let live = true;
    archReadFile(root, path).then(
      (data) => live && setState({ key, kind: "ok", data }),
      (e) => live && setState({ key, kind: "error", msg: errMsg(e) }),
    );
    return () => { live = false; };
  }, [root, path, key]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  // 只認「這個檔案」的結果，換檔的瞬間不會閃出上一個檔案的內容。
  const cur = state && state.key === key ? state : null;
  useEffect(() => {
    if (cur?.kind === "ok") hitRef.current?.scrollIntoView?.({ block: "center" });
  }, [cur, line]);

  const copy = () => {
    const abs = `${root.replace(/[\\/]+$/, "")}/${path}`;
    try {
      void navigator.clipboard?.writeText(abs)?.then(() => { setCopied(true); setTimeout(() => setCopied(false), 1500); }, () => {});
    } catch { /* 剪貼簿不可用就算了 */ }
  };
  const open = () => { archOpenPath(root, path).catch((e) => setState({ key, kind: "error", msg: errMsg(e) })); };

  let body;
  if (!cur) {
    body = <div className="arch-pv-note">{t.arch_pv_loading}</div>;
  } else if (cur.kind === "error") {
    body = <div className="arch-pv-note arch-pv-error" role="alert">{cur.msg}</div>;
  } else {
    const lines = cur.data.content.split(/\r?\n/);
    if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
    const shown = lines.slice(0, MAX_LINES);
    const kb = Math.round(new TextEncoder().encode(cur.data.content).length / 1024);
    body = (
      <>
        {cur.data.truncated && <div className="arch-pv-note">{t.arch_pv_truncated.replace("{n}", String(kb))}</div>}
        <div className="arch-pv-code">
          {shown.map((text, i) => {
            const n = i + 1;
            const hit = n === line;
            return (
              <div key={n} data-line={n} ref={hit ? hitRef : undefined} aria-current={hit ? "true" : undefined}
                className={`arch-pv-row ${hit ? "arch-pv-hit" : ""}`}>
                <span className="arch-pv-no" aria-hidden="true">{n}</span>
                <span className="arch-pv-text">{text || "​"}</span>
              </div>
            );
          })}
        </div>
        {lines.length > MAX_LINES && <div className="arch-pv-note">{t.arch_pv_more_lines.replace("{n}", String(MAX_LINES))}</div>}
      </>
    );
  }

  return (
    <div className="arch-preview" role="dialog" aria-label={t.arch_pv_title}>
      <div className="arch-pv-head">
        <code className="arch-pv-path" title={path}>{path}</code>
        <button type="button" className="arch-btn arch-btn-small" onClick={copy}>{copied ? t.arch_pv_copied : t.arch_pv_copy}</button>
        <button type="button" className="arch-btn arch-btn-small" onClick={open}>{t.arch_pv_open}</button>
        <button type="button" className="arch-btn arch-btn-small" aria-label={t.arch_pv_close} onClick={onClose}>✕</button>
      </div>
      <div className="arch-pv-body">{body}</div>
    </div>
  );
}
