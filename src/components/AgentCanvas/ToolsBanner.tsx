import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { addClaudeToPath, type ToolsCheck } from "../../ipc/agents";
import { installCommand, type Tool } from "./toolsCheck";

/** 協作頁頂列下方：這台電腦缺哪些工具、各平台怎麼裝。 */
export function ToolsBanner({
  check,
  missing,
  busy,
  onRecheck,
}: {
  check: ToolsCheck;
  missing: Tool[];
  busy: boolean;
  onRecheck: () => void;
}) {
  const { t } = useLocale();
  const [copied, setCopied] = useState<Tool | null>(null);
  const copy = async (tool: Tool, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(tool);
      window.setTimeout(() => setCopied((c) => (c === tool ? null : c)), 1500);
    } catch {
      // 複製失敗也看得到指令，可以自己選取。
    }
  };
  const [pathBusy, setPathBusy] = useState(false);
  const [pathError, setPathError] = useState<string | null>(null);
  const addToPath = async () => {
    setPathBusy(true);
    setPathError(null);
    try {
      await addClaudeToPath();
      onRecheck();
    } catch (e) {
      setPathError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    } finally {
      setPathBusy(false);
    }
  };
  const what: Record<Tool, string> = { git: t.ac_tool_git, claude: t.ac_tool_claude, codex: t.ac_tool_codex };
  return (
    <div className="ac-tools" data-testid="ac-tools" role="alert">
      <div className="ac-tools-head">
        <span>⚠ {t.ac_tools_title}</span>
        <span className="ac-spacer" />
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={onRecheck}>
          {busy ? t.ac_tools_checking : t.ac_tools_recheck}
        </button>
      </div>
      {missing.map((tool) => {
        const cmd = installCommand(tool, check.platform);
        // 裝好了、只是不在 PATH：不要再叫人重裝，給路徑與（Windows）一鍵加入。
        if (tool === "claude" && check.claude_unlisted) {
          return (
            <div key={tool} className="ac-tools-row" data-testid="ac-tools-unlisted">
              <span className="ac-tools-what">{t.ac_tools_unlisted(check.claude_unlisted)}</span>
              {check.platform === "windows" ? (
                <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={pathBusy} onClick={() => void addToPath()}>
                  {pathBusy ? t.ac_tools_checking : t.ac_tools_add_path}
                </button>
              ) : (
                <span className="ac-tools-note">{t.ac_tools_unlisted_manual}</span>
              )}
              {pathError && <span className="ac-tools-note" role="alert">{pathError}</span>}
            </div>
          );
        }
        return (
          <div key={tool} className="ac-tools-row">
            <span className="ac-tools-what">{what[tool]}</span>
            <code className="ac-tools-cmd">{cmd}</code>
            <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => void copy(tool, cmd)}>
              {copied === tool ? t.ac_tools_copied : t.ac_tools_copy}
            </button>
            {tool === "claude" && check.claude_command.trim() !== "claude" && (
              <span className="ac-tools-note">{t.ac_tools_custom_command(check.claude_command)}</span>
            )}
          </div>
        );
      })}
      {check.platform === "windows" && <div className="ac-tools-note">{t.ac_tools_restart_windows}</div>}
    </div>
  );
}
