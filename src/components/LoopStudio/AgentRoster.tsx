import { useState, useRef, useEffect, useCallback } from "react";
import type { ProviderInfo } from "../../ipc/provider";
import type { OrchestratorAgent } from "../../hooks/useOrchestratorLoop";
import type { AgentToolName } from "../../hooks/useSubAgentLoop";
import { invokeAiChat } from "../../ipc/ai";

const ALL_TOOLS: AgentToolName[] = ["read_file", "write_file", "list_directory", "execute_command"];
const TOOL_LABELS: Record<AgentToolName, string> = {
  read_file: "讀檔",
  write_file: "寫檔",
  list_directory: "列目錄",
  execute_command: "執行指令",
};

interface AgentPreset {
  label: string;
  emoji: string;
  name: string;
  roleDescription: string;
  tools: AgentToolName[];
}

const AGENT_PRESETS: AgentPreset[] = [
  {
    label: "程式碼工程師",
    emoji: "💻",
    name: "Coder",
    roleDescription: "你是一位資深全端工程師，負責根據需求撰寫、修改程式碼。優先考慮程式碼品質、可讀性與維護性。完成後回報修改的檔案與摘要。",
    tools: ["read_file", "write_file", "list_directory", "execute_command"],
  },
  {
    label: "測試工程師",
    emoji: "🧪",
    name: "Tester",
    roleDescription: "你是一位測試工程師，負責撰寫單元測試與整合測試，並執行測試套件以確保程式碼品質。回報測試結果與覆蓋率摘要。",
    tools: ["read_file", "write_file", "execute_command"],
  },
  {
    label: "程式碼審查員",
    emoji: "🔍",
    name: "Reviewer",
    roleDescription: "你是一位資深工程師，負責審查程式碼品質，找出潛在 bug、安全漏洞與效能問題，並給出具體且可執行的改善建議。",
    tools: ["read_file", "list_directory"],
  },
  {
    label: "研究分析師",
    emoji: "📊",
    name: "Researcher",
    roleDescription: "你是一位研究分析師，負責閱讀並分析程式碼庫、文件與設定檔，整理出有條理的報告與發現，供其他 agent 參考使用。",
    tools: ["read_file", "list_directory"],
  },
  {
    label: "DevOps 工程師",
    emoji: "⚙️",
    name: "DevOps",
    roleDescription: "你是一位 DevOps 工程師，負責執行建置、測試、部署相關的 shell 指令，解讀輸出結果，並回報執行狀態與任何錯誤。",
    tools: ["execute_command", "read_file", "write_file"],
  },
  {
    label: "文件撰寫員",
    emoji: "📝",
    name: "DocWriter",
    roleDescription: "你是一位技術文件撰寫員，負責根據程式碼產生清晰的 API 文件、README、使用說明與架構說明，使用繁體中文撰寫。",
    tools: ["read_file", "write_file", "list_directory"],
  },
  {
    label: "重構專家",
    emoji: "🔧",
    name: "Refactorer",
    roleDescription: "你是一位重構專家，負責改善現有程式碼的結構、消除重複邏輯、提升可讀性，同時確保功能不變。每次修改後執行測試驗證。",
    tools: ["read_file", "write_file", "list_directory", "execute_command"],
  },
  {
    label: "安全審計員",
    emoji: "🛡️",
    name: "SecurityAuditor",
    roleDescription: "你是一位資安工程師，負責審查程式碼中的 OWASP Top 10 漏洞、硬編碼金鑰、注入風險與不安全的依賴套件，並給出修復建議。",
    tools: ["read_file", "list_directory"],
  },
  {
    label: "架構師",
    emoji: "🏛️",
    name: "Architect",
    roleDescription: "你是一位資深軟體架構師，負責分析現有程式碼庫的架構，評估模組邊界、依賴關係與設計模式，提出可落地的架構改善方案、介面定義與重構路線圖，並考量可擴展性、可維護性與跨平台相容性。回報時需包含具體的架構圖描述（文字形式）與優先順序建議。",
    tools: ["read_file", "list_directory"],
  },
];

interface AgentRosterProps {
  agents: OrchestratorAgent[];
  providers: ProviderInfo[];
  onChange: (agents: OrchestratorAgent[]) => void;
}

export function AgentRoster({ agents, providers, onChange }: AgentRosterProps) {
  const [expanded, setExpanded] = useState<number | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [generatingIdx, setGeneratingIdx] = useState<number | null>(null);
  const pickerRef = useRef<HTMLDivElement>(null);

  const handleGenerateDescription = useCallback(async (idx: number) => {
    const agent = agents[idx];
    if (!agent.name.trim()) return;
    setGeneratingIdx(idx);
    try {
      const reply = await invokeAiChat(
        [
          {
            role: "system",
            content: "你是一位 AI 系統設計師，專門為 AI Agent 撰寫角色描述。根據角色名稱，產生清晰、具體、可執行的角色描述，說明核心職責、工作方式與產出格式。約 2-3 句話，使用繁體中文。只輸出角色描述，不加任何前言或說明。",
          },
          {
            role: "user",
            content: `角色名稱：${agent.name}\n請為這個角色產生描述。`,
          },
        ],
        "roster-agent-gen",
        providers[0]?.id,
      );
      const desc = reply.content?.trim();
      if (desc) {
        onChange(agents.map((a, i) => i === idx ? { ...a, roleDescription: desc } : a));
      }
    } catch (err) {
      console.error("Generate description failed:", err);
    } finally {
      setGeneratingIdx(null);
    }
  }, [agents, providers, onChange]);

  useEffect(() => {
    if (!pickerOpen) return;
    const handler = (e: MouseEvent) => {
      if (pickerRef.current && !pickerRef.current.contains(e.target as Node)) {
        setPickerOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [pickerOpen]);

  const addAgent = (preset?: AgentPreset) => {
    const agent: OrchestratorAgent = {
      name: preset?.name ?? "",
      providerId: providers[0]?.id ?? "",
      roleDescription: preset?.roleDescription ?? "",
      tools: preset?.tools ?? ["read_file", "write_file", "list_directory", "execute_command"],
    };
    const next = [...agents, agent];
    onChange(next);
    setExpanded(next.length - 1);
    setPickerOpen(false);
  };

  const update = (idx: number, patch: Partial<OrchestratorAgent>) => {
    onChange(agents.map((a, i) => i === idx ? { ...a, ...patch } : a));
  };

  const toggleTool = (idx: number, tool: AgentToolName) => {
    const agent = agents[idx];
    const has = agent.tools.includes(tool);
    update(idx, { tools: has ? agent.tools.filter(t => t !== tool) : [...agent.tools, tool] });
  };

  return (
    <div className="ls-roster">
      <div className="ls-roster-header">
        <span className="ls-section-label">Agent Roster</span>
        <div className="ls-add-btn-wrap" ref={pickerRef}>
          <button
            type="button"
            className="ls-add-btn"
            onClick={() => setPickerOpen(p => !p)}
          >
            + Agent ▾
          </button>
          {pickerOpen && (
            <div className="ls-preset-picker">
              {AGENT_PRESETS.map(preset => (
                <button
                  key={preset.name}
                  type="button"
                  className="ls-preset-item"
                  onClick={() => addAgent(preset)}
                >
                  <span className="ls-preset-emoji">{preset.emoji}</span>
                  <span className="ls-preset-label">{preset.label}</span>
                </button>
              ))}
              <div className="ls-preset-divider" />
              <button
                type="button"
                className="ls-preset-item ls-preset-custom"
                onClick={() => addAgent()}
              >
                <span className="ls-preset-emoji">✏️</span>
                <span className="ls-preset-label">自訂（空白）</span>
              </button>
            </div>
          )}
        </div>
      </div>

      {agents.length === 0 && (
        <div className="ls-roster-empty">尚無 Agent，點擊「+ Agent」從預設角色或自訂新增</div>
      )}

      {agents.map((agent, idx) => (
        <div key={idx} className="ls-agent-card">
          <div
            className="ls-agent-card-header"
            onClick={() => setExpanded(expanded === idx ? null : idx)}
          >
            <span className="ls-agent-name">{agent.name || "(未命名)"}</span>
            <span className="ls-agent-provider">
              {providers.find(p => p.id === agent.providerId)?.display_name ?? "—"}
            </span>
            <button
              type="button"
              className="ls-agent-remove"
              onClick={e => { e.stopPropagation(); onChange(agents.filter((_, i) => i !== idx)); }}
            >
              ×
            </button>
          </div>

          {expanded === idx && (
            <div className="ls-agent-body">
              <label className="ls-field">
                <span>名稱</span>
                <input
                  value={agent.name}
                  onChange={e => update(idx, { name: e.target.value })}
                  placeholder="例：Coder"
                />
              </label>
              <label className="ls-field">
                <span>Provider</span>
                <select value={agent.providerId} onChange={e => update(idx, { providerId: e.target.value })}>
                  {providers.map(p => (
                    <option key={p.id} value={p.id}>{p.display_name}</option>
                  ))}
                </select>
              </label>
              <div className="ls-field">
                <div className="ls-field-label-row">
                  <span>角色描述</span>
                  <button
                    type="button"
                    className="ls-enhance-btn"
                    onClick={() => handleGenerateDescription(idx)}
                    disabled={!agent.name.trim() || generatingIdx !== null}
                    title={agent.name.trim() ? "依角色名稱 AI 產生描述" : "請先輸入角色名稱"}
                  >
                    {generatingIdx === idx ? "⏳ 產生中…" : "✨ AI 產生"}
                  </button>
                </div>
                <textarea
                  value={agent.roleDescription}
                  onChange={e => update(idx, { roleDescription: e.target.value })}
                  placeholder="例：你是一個專業的程式碼重構工程師，負責改善程式碼品質"
                  rows={3}
                  disabled={generatingIdx === idx}
                />
              </div>
              <div className="ls-field">
                <span>工具</span>
                <div className="ls-tool-checks">
                  {ALL_TOOLS.map(tool => (
                    <label key={tool} className="ls-tool-check">
                      <input
                        type="checkbox"
                        checked={agent.tools.includes(tool)}
                        onChange={() => toggleTool(idx, tool)}
                      />
                      {TOOL_LABELS[tool]}
                    </label>
                  ))}
                </div>
              </div>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
