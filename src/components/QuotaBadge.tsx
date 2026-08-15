import type { QuotaWindow } from "../ipc/usage";
import "./QuotaBadge.css";

interface Props {
  window: QuotaWindow;
  /**
   * 是否畫迷你長條。預設 true。
   *
   * 設定頁的配額卡自己有一條全寬長條，那裡要傳 false，否則同一個數字會被
   * 畫兩次。
   */
  bar?: boolean;
}

/** 把重置時間寫成人看得懂的相對說法。過期或沒給就回 null。 */
function resetsInText(resetsAt: number | null): string | null {
  if (resetsAt === null) return null;
  const secs = resetsAt - Math.floor(Date.now() / 1000);
  if (secs <= 0) return null;
  if (secs < 3600) return `${Math.ceil(secs / 60)} 分鐘後重置`;
  if (secs < 86400) return `${Math.round(secs / 3600)} 小時後重置`;
  return `${Math.round(secs / 86400)} 天後重置`;
}

export function QuotaBadge({ window: w, bar = true }: Props) {
  // 這個徽章擠在標題列裡，寬度是稀缺資源：只放最能據以行動的那個數字。
  // 剩餘次數（Copilot）比百分比精確，優先用它；時間窗則用「窗長 + 百分比」。
  // 完整脈絡（窗別、百分比、重置時間）放 tooltip。
  const text = w.detail ?? `${w.label} ${Math.round(w.used_percent)}%`;

  const tip = [
    w.detail ? `${w.label}：剩 ${w.detail}` : w.label,
    `已用 ${Math.round(w.used_percent)}%`,
    resetsInText(w.resets_at),
  ]
    .filter(Boolean)
    .join(" · ");

  // 夾在 0–100：上游 burst 後可能短暫回超出範圍的值，長條不該畫出軌道外。
  const fill = Math.min(100, Math.max(0, w.used_percent));

  return (
    <span
      className={`quota-badge quota-badge--${w.severity}`}
      data-testid="quota-badge"
      title={tip}
    >
      {bar && (
        <span className="quota-bar" aria-hidden="true">
          <span
            className="quota-bar-fill"
            data-testid="quota-bar-fill"
            style={{ width: `${fill}%` }}
          />
        </span>
      )}
      {text}
    </span>
  );
}
