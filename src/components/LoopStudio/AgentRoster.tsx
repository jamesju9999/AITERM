import { useState, useRef, useEffect, useCallback } from "react";
import type { ProviderInfo } from "../../ipc/provider";
import type { OrchestratorAgent } from "../../hooks/useOrchestratorLoop";
import type { AgentToolName } from "../../hooks/useSubAgentLoop";
import { invokeAiChat } from "../../ipc/ai";
import { useLocale } from "../../contexts/LocaleContext";
import { ModelPickerButton } from "../ModelPickerButton";

const ALL_TOOLS: AgentToolName[] = ["read_file", "write_file", "list_directory", "execute_command"];

interface AgentPreset {
  label: string;
  emoji: string;
  name: string;
  roleDescription: string;
  tools: AgentToolName[];
}

interface AgentRosterProps {
  agents: OrchestratorAgent[];
  providers: ProviderInfo[];
  onChange: (agents: OrchestratorAgent[]) => void;
}

export function AgentRoster({ agents, providers, onChange }: AgentRosterProps) {
  const { t } = useLocale();

  const AGENT_PRESETS: AgentPreset[] = [
    {
      label: t.ls_preset_coder,
      emoji: "💻",
      name: "Coder",
      roleDescription: t.ls_preset_coder_desc,
      tools: ["read_file", "write_file", "list_directory", "execute_command"],
    },
    {
      label: t.ls_preset_tester,
      emoji: "🧪",
      name: "Tester",
      roleDescription: t.ls_preset_tester_desc,
      tools: ["read_file", "write_file", "execute_command"],
    },
    {
      label: t.ls_preset_reviewer,
      emoji: "🔍",
      name: "Reviewer",
      roleDescription: t.ls_preset_reviewer_desc,
      tools: ["read_file", "list_directory"],
    },
    {
      label: t.ls_preset_researcher,
      emoji: "📊",
      name: "Researcher",
      roleDescription: t.ls_preset_researcher_desc,
      tools: ["read_file", "list_directory"],
    },
    {
      label: t.ls_preset_devops,
      emoji: "⚙️",
      name: "DevOps",
      roleDescription: t.ls_preset_devops_desc,
      tools: ["execute_command", "read_file", "write_file"],
    },
    {
      label: t.ls_preset_docs,
      emoji: "📝",
      name: "DocWriter",
      roleDescription: t.ls_preset_docs_desc,
      tools: ["read_file", "write_file", "list_directory"],
    },
    {
      label: t.ls_preset_refactorer,
      emoji: "🔧",
      name: "Refactorer",
      roleDescription: t.ls_preset_refactorer_desc,
      tools: ["read_file", "write_file", "list_directory", "execute_command"],
    },
    {
      label: t.ls_preset_security,
      emoji: "🛡️",
      name: "SecurityAuditor",
      roleDescription: t.ls_preset_security_desc,
      tools: ["read_file", "list_directory"],
    },
    {
      label: t.ls_preset_architect,
      emoji: "🏛️",
      name: "Architect",
      roleDescription: t.ls_preset_architect_desc,
      tools: ["read_file", "list_directory"],
    },
  ];

  const TOOL_LABELS: Record<AgentToolName, string> = {
    read_file: t.ls_tool_read_file,
    write_file: t.ls_tool_write_file,
    list_directory: t.ls_tool_list_dir,
    execute_command: t.ls_tool_execute_cmd,
  };
  const PRESET_LABELS: Record<string, string> = {
    Coder: t.ls_preset_coder,
    Tester: t.ls_preset_tester,
    Reviewer: t.ls_preset_reviewer,
    Researcher: t.ls_preset_researcher,
    DevOps: t.ls_preset_devops,
    DocWriter: t.ls_preset_docs,
    Refactorer: t.ls_preset_refactorer,
    SecurityAuditor: t.ls_preset_security,
    Architect: t.ls_preset_architect,
  };
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
            content: t.ls_role_generator_system,
          },
          {
            role: "user",
            content: t.ls_role_generator_user(agent.name),
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
  }, [agents, providers, onChange, t]);

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
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
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
                  <span className="ls-preset-label">{PRESET_LABELS[preset.name] ?? preset.label}</span>
                </button>
              ))}
              <div className="ls-preset-divider" />
              <button
                type="button"
                className="ls-preset-item ls-preset-custom"
                onClick={() => addAgent()}
              >
                <span className="ls-preset-emoji">✏️</span>
                <span className="ls-preset-label">{t.ls_preset_custom}</span>
              </button>
            </div>
          )}
        </div>
      </div>

      {agents.length === 0 && (
        <div className="ls-roster-empty">{t.ls_roster_empty}</div>
      )}

      {agents.map((agent, idx) => (
        <div key={idx} className="ls-agent-card">
          <div
            className="ls-agent-card-header"
            onClick={() => setExpanded(expanded === idx ? null : idx)}
          >
            <span className="ls-agent-name">{agent.name || t.ls_agent_unnamed}</span>
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
                <span>{t.ls_field_agent_name}</span>
                <input
                  value={agent.name}
                  onChange={e => update(idx, { name: e.target.value })}
                  placeholder={t.ls_agent_name_placeholder}
                />
              </label>
              <label className="ls-field">
                <span>Provider</span>
                <ModelPickerButton
                  providers={providers}
                  selectedId={agent.providerId}
                  onChange={(id) => update(idx, { providerId: id })}
                />
              </label>
              <div className="ls-field">
                <div className="ls-field-label-row">
                  <span>{t.ls_field_role_desc}</span>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    onClick={() => handleGenerateDescription(idx)}
                    disabled={!agent.name.trim() || generatingIdx !== null}
                    title={agent.name.trim() ? t.ls_generate_desc_title_ready : t.ls_generate_desc_title_empty}
                  >
                    {generatingIdx === idx ? t.ls_generating_desc_btn : t.ls_generate_desc_btn}
                  </button>
                </div>
                <textarea
                  value={agent.roleDescription}
                  onChange={e => update(idx, { roleDescription: e.target.value })}
                  placeholder={t.ls_role_desc_placeholder}
                  rows={3}
                  disabled={generatingIdx === idx}
                />
              </div>
              <div className="ls-field">
                <span>{t.ls_field_tools}</span>
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
