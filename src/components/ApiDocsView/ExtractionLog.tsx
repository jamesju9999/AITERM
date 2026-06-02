// src/components/ApiDocsView/ExtractionLog.tsx
import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { ApiDocsLogEvent } from "../../ipc/apiDocs";

interface Props {
  current: number;
  total: number;
  logs: ApiDocsLogEvent[];
  outputFiles: string[];
}

export function ExtractionLog({ current, total, logs, outputFiles }: Props) {
  const { t } = useLocale();
  const pct = total > 0 ? Math.round((current / total) * 100) : 0;
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  return (
    <div className="extraction-log">
      {total > 0 && (
        <div className="extraction-log__progress">
          <div
            className="extraction-log__bar-track"
          >
            <div
              role="progressbar"
              aria-valuenow={current}
              aria-valuemin={0}
              aria-valuemax={total}
              className="extraction-log__bar-fill"
              style={{ width: `${pct}%` }}
            />
          </div>
          <span className="extraction-log__bar-label">
            {current} / {total}
          </span>
        </div>
      )}

      <div className="extraction-log__list">
        {logs.map((entry, i) => (
          <div
            key={i}
            className={`extraction-log__entry extraction-log__entry--${entry.level}`}
          >
            {entry.message}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>

      {outputFiles.length > 0 && (
        <div className="extraction-log__output">
          <div className="extraction-log__output-title">{t.api_docs_output_files}</div>
          {outputFiles.map((f) => (
            <div key={f} className="extraction-log__output-file">
              {f}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
