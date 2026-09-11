import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { detectPowerShell7 } from "../../ipc/shell";
import type { ShellIdentity } from "../../hooks/useShellIdentity";
import "./index.css";

interface Props {
  identity: ShellIdentity | null;
}

/** Windows PowerShell 5.1 回報的 edition 值。7.x 回 "Core"。 */
const LEGACY_EDITION = "Desktop";

/**
 * 只在這個分頁跑的是 Windows PowerShell 5.1 時才出現的警告徽章。
 *
 * 5.1 算全形字寬度有誤，dir 這類表格輸出的每一列都會溢出換行。使用者無從
 * 得知跑的不是 PowerShell 7——實機上發生過「明明裝了 7.6.6，AITerm 啟動時
 * 的 PATH 卻找不到 pwsh.exe」，因為畫面上沒有任何地方顯示實際跑的是哪一個
 * shell，查了很久才發現。
 */
export function ShellWarningBadge({ identity }: Props) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [pwsh7Path, setPwsh7Path] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const isLegacy = identity?.shell === "PowerShell" && identity.edition === LEGACY_EDITION;

  useEffect(() => {
    // 只有真的要顯示徽章時才問後端——正常情況一次都不會呼叫。
    if (!isLegacy) return;
    let alive = true;
    void detectPowerShell7().then((p) => {
      if (alive) setPwsh7Path(p);
    });
    return () => {
      alive = false;
    };
  }, [isLegacy]);

  if (!isLegacy) return null;

  async function onCopy() {
    await navigator.clipboard.writeText(t.shell_badge_install_command);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  return (
    <span className="aiterm-shellwarn">
      <button
        className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm aiterm-shellwarn__btn"
        title={t.shell_badge_title}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
      >
        <span>⚠ {t.shell_badge_legacy_powershell}</span>
      </button>

      {open && (
        <div className="aiterm-shellwarn__panel" onClick={(e) => e.stopPropagation()}>
          <div className="aiterm-shellwarn__title">{t.shell_badge_title}</div>
          <div className="aiterm-shellwarn__why">{t.shell_badge_why}</div>

          {pwsh7Path ? (
            <div className="aiterm-shellwarn__body">
              <div>{t.shell_badge_found_intro(pwsh7Path)}</div>
              <div className="aiterm-shellwarn__action">{t.shell_badge_found_action}</div>
            </div>
          ) : (
            <div className="aiterm-shellwarn__body">
              <div>{t.shell_badge_missing_intro}</div>
              <div className="aiterm-shellwarn__cmdrow">
                <code className="aiterm-shellwarn__cmd">{t.shell_badge_install_command}</code>
                <button
                  className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                  onClick={() => void onCopy()}
                >
                  {copied ? t.shell_badge_copied : t.shell_badge_copy}
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </span>
  );
}
