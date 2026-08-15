import type { QuotaWindow } from "../ipc/usage";
import "./QuotaBadge.css";

interface Props {
  window: QuotaWindow;
}

export function QuotaBadge({ window: w }: Props) {
  // detail 保留了上游的原始語意（Copilot 的次數），比百分比精確。
  const value = w.detail ?? `${Math.round(w.used_percent)}%`;
  return (
    <span className={`quota-badge quota-badge--${w.severity}`} data-testid="quota-badge">
      {w.label} {value}
    </span>
  );
}
