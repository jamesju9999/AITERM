import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { usageSummary, type UsageSummaryEntry } from "../../ipc/usage";

type State =
  | { kind: "loading" }
  | { kind: "ready"; entries: UsageSummaryEntry[] }
  | { kind: "failed" };

export function UsageSection() {
  const { t } = useLocale();
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    usageSummary("today")
      .then((entries) => { if (!cancelled) setState({ kind: "ready", entries }); })
      .catch(() => { if (!cancelled) setState({ kind: "failed" }); });
    return () => { cancelled = true; };
  }, []);

  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_usage_title}</h2>
      {/* 查不到就說查不到。顯示 0 會讓使用者以為自己今天沒用過。 */}
      {state.kind === "failed" && <p className="home-empty">{t.home_usage_failed}</p>}
      {state.kind === "ready" && state.entries.length === 0 && (
        <p className="home-empty">{t.home_usage_empty}</p>
      )}
      {state.kind === "ready" && state.entries.length > 0 && (
        <table className="home-usage-table">
          <thead>
            <tr>
              <th>{t.home_usage_model}</th>
              <th>{t.home_usage_requests}</th>
              <th>{t.home_usage_tokens}</th>
            </tr>
          </thead>
          <tbody>
            {state.entries.map((e) => (
              <tr key={`${e.provider_id}/${e.model}`}>
                <td>{e.model}</td>
                <td>{e.requests}</td>
                <td>{(e.prompt_tokens + e.completion_tokens).toLocaleString()}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
