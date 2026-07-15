import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { createPty, closePty } from "../../ipc/pty";
import { pickFolder } from "../../ipc/vcs";
import { useOrchestratorLoop, type OrchestratorAgent, type LoopConfig } from "../../hooks/useOrchestratorLoop";
import { AgentRoster } from "./AgentRoster";
import { ExecutionTrace } from "./ExecutionTrace";
import { validateRoster, type ValidationIssue } from "./validateRoster";
import { SessionPicker } from "./SessionPicker";
import { loopSessionLoad, parseLoopSessionData, loopProjectPickOpen, loopProjectPickSave } from "../../ipc/loopSession";
import { readFile, writeTextFile } from "../../ipc/fs";
import { invokeAiChat } from "../../ipc/ai";
import { useLocale } from '../../contexts/LocaleContext';
import "./styles.css";

const STORAGE_KEY = "aiterm-loop-studio-roster";

interface RosterState {
  orchestratorProvider: string;
  verifierProvider: string;
  orchestratorName: string;
  verifierName: string;
  subAgents: OrchestratorAgent[];
  goal: string;
  stoppingCondition: string;
  maxLoops: number;
  maxOrchestratorSteps: number;
  maxInnerIterations: number;
  projectDir: string;
  fullAuto: boolean;
}

function loadRoster(): RosterState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as RosterState;
  } catch {}
  return {
    orchestratorProvider: "",
    verifierProvider: "",
    orchestratorName: "Orchestrator",
    verifierName: "Verifier",
    subAgents: [],
    goal: "",
    stoppingCondition: "",
    maxLoops: 5,
    maxOrchestratorSteps: 40,
    maxInnerIterations: 30,
    projectDir: "",
    fullAuto: false,
  };
}

function saveRoster(state: RosterState) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {}
}

interface LoopStudioViewProps {
  sessionId?: string;
  tabId?: string;
  registerCloseGuard?: (tabId: string, guard: () => Promise<boolean>) => void;
  unregisterCloseGuard?: (tabId: string) => void;
}

export function LoopStudioView({
  sessionId: externalSessionId,
  tabId,
  registerCloseGuard,
  unregisterCloseGuard,
}: LoopStudioViewProps) {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [roster, setRoster] = useState<RosterState>(loadRoster);
  const [ptySessionId, setPtySessionId] = useState<string | null>(null);
  const [warningsDismissed, setWarningsDismissed] = useState(false);
  const [enhancingField, setEnhancingField] = useState<"goal" | "stopping" | null>(null);
  const [prevText, setPrevText] = useState<{ goal?: string; stopping?: string }>({});
  const [currentProjectPath, setCurrentProjectPath] = useState<string | null>(null);
  const [isDirty, setIsDirty] = useState(false);
  const isDirtyRef = useRef(false);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const closeResolveRef = useRef<((canClose: boolean) => void) | null>(null);

  const { t } = useLocale();
  const loop = useOrchestratorLoop();
  const timingMode = (localStorage.getItem("loopTimingMode") ?? "compact") as "full" | "compact";

  useEffect(() => {
    listProviders().then(setProviders).catch(() => setProviders([]));
  }, []);

  // Create a dedicated PTY for tool execution; recreate when projectDir changes
  useEffect(() => {
    if (externalSessionId) {
      setPtySessionId(externalSessionId);
      return;
    }
    let id: string | null = null;
    createPty({ rows: 40, cols: 120 }, roster.projectDir || undefined).then(newId => {
      id = newId;
      setPtySessionId(newId);
    }).catch(() => {});
    return () => { if (id) closePty(id).catch(() => {}); };
  }, [externalSessionId, roster.projectDir]);

  // Set default providers once loaded
  useEffect(() => {
    if (providers.length === 0) return;
    setRoster(prev => ({
      ...prev,
      orchestratorProvider: prev.orchestratorProvider || providers[0]?.id || "",
      verifierProvider: prev.verifierProvider || providers[0]?.id || "",
    }));
  }, [providers]);

  // Re-run validation whenever roster or goal changes; reset dismissed state
  const validationIssues = useMemo<ValidationIssue[]>(() => {
    return validateRoster(
      roster.subAgents,
      roster.orchestratorProvider,
      roster.verifierProvider,
      roster.goal,
    );
  }, [roster.subAgents, roster.orchestratorProvider, roster.verifierProvider, roster.goal]);

  const hasErrors = validationIssues.some(i => i.level === "error");
  const hasWarnings = validationIssues.some(i => i.level === "warning");
  const warnings = validationIssues.filter(i => i.level === "warning");
  const errors = validationIssues.filter(i => i.level === "error");

  // Reset dismissed state when issues change
  useEffect(() => { setWarningsDismissed(false); }, [validationIssues]);

  const markDirty = useCallback(() => {
    isDirtyRef.current = true;
    setIsDirty(true);
  }, []);

  const markClean = useCallback(() => {
    isDirtyRef.current = false;
    setIsDirty(false);
  }, []);

  // Register async close guard — shows custom modal instead of window.confirm
  useEffect(() => {
    if (!tabId || !registerCloseGuard) return;
    registerCloseGuard(tabId, () => {
      if (loop.isRunning) {
        return new Promise<boolean>(resolve => {
          closeResolveRef.current = (canClose: boolean) => {
            if (canClose) loop.stop();
            resolve(canClose);
          };
          setShowCloseConfirm(true);
        });
      }
      const hasContent = isDirtyRef.current &&
        (roster.goal.trim() !== "" || roster.subAgents.length > 0);
      if (!hasContent) return Promise.resolve(true);
      return new Promise<boolean>(resolve => {
        closeResolveRef.current = resolve;
        setShowCloseConfirm(true);
      });
    });
    return () => { unregisterCloseGuard?.(tabId); };
  }, [tabId, registerCloseGuard, unregisterCloseGuard, roster.goal, roster.subAgents.length, loop.isRunning, loop.stop]);

  const updateRoster = useCallback((patch: Partial<RosterState>) => {
    setRoster(prev => {
      const next = { ...prev, ...patch };
      saveRoster(next);
      return next;
    });
    markDirty();
  }, [markDirty]);

  const handleStart = useCallback(() => {
    if (!ptySessionId) return;
    if (!roster.goal.trim()) return;
    if (hasErrors) return;
    if (hasWarnings && !warningsDismissed) return;

    const orchestrator: OrchestratorAgent = {
      name: roster.orchestratorName,
      providerId: roster.orchestratorProvider,
      roleDescription: "You are the Orchestrator. Coordinate sub-agents to achieve the goal.",
      tools: [],
      isOrchestrator: true,
    };
    const verifier: OrchestratorAgent = {
      name: roster.verifierName,
      providerId: roster.verifierProvider,
      roleDescription: "You are the Verifier. Evaluate whether the goal has been achieved.",
      tools: [],
      isVerifier: true,
    };

    const config: LoopConfig = {
      goal: roster.goal,
      stoppingCondition: roster.stoppingCondition || roster.goal,
      orchestrator,
      verifier,
      subAgents: roster.subAgents,
      maxLoops: roster.maxLoops,
      maxOrchestratorSteps: roster.maxOrchestratorSteps,
      maxInnerIterations: roster.maxInnerIterations,
      sessionId: ptySessionId,
      projectDir: roster.projectDir || undefined,
      fullAuto: roster.fullAuto ?? false,
    };

    void loop.start(config);
  }, [loop, ptySessionId, roster]);

  const handlePickFolder = useCallback(async () => {
    const folder = await pickFolder().catch(() => null);
    if (folder) updateRoster({ projectDir: folder });
  }, [updateRoster]);

  const handleEnhance = useCallback(async (field: "goal" | "stopping") => {
    const text = field === "goal" ? roster.goal : roster.stoppingCondition;
    if (!text.trim()) return;
    setEnhancingField(field);
    try {
      const fieldLabel = field === "goal" ? "任務目標" : "停止條件";
      const reply = await invokeAiChat(
        [
          {
            role: "system",
            content: "你是一位專業的 AI 提示詞工程師。使用者提供一段粗略描述，你的任務是強化並潤飾文字，使其更具體、明確、可衡量，且適合作為 AI Agent 的執行指引。保留原意，不要過度延伸範圍。只輸出潤飾後的文字，不加任何前言或說明。使用繁體中文。",
          },
          {
            role: "user",
            content: `請潤飾這個${fieldLabel}：\n\n${text}`,
          },
        ],
        ptySessionId ?? "loop-enhance",
        roster.orchestratorProvider || undefined,
      );
      const enhanced = reply.content?.trim();
      if (enhanced) {
        setPrevText(prev => ({ ...prev, [field]: text }));
        if (field === "goal") updateRoster({ goal: enhanced });
        else updateRoster({ stoppingCondition: enhanced });
      }
    } catch (err) {
      console.error("Enhance failed:", err);
    } finally {
      setEnhancingField(null);
    }
  }, [roster.goal, roster.stoppingCondition, roster.orchestratorProvider, ptySessionId, updateRoster]);

  const handleUndo = useCallback((field: "goal" | "stopping") => {
    const prev = prevText[field];
    if (!prev) return;
    if (field === "goal") updateRoster({ goal: prev });
    else updateRoster({ stoppingCondition: prev });
    setPrevText(p => ({ ...p, [field]: undefined }));
  }, [prevText, updateRoster]);

  const handleNewProject = useCallback(() => {
    const defaults: RosterState = {
      orchestratorProvider: providers[0]?.id ?? "",
      verifierProvider: providers[0]?.id ?? "",
      orchestratorName: "Orchestrator",
      verifierName: "Verifier",
      subAgents: [],
      goal: "",
      stoppingCondition: "",
      maxLoops: 5,
      maxOrchestratorSteps: 40,
      maxInnerIterations: 30,
      projectDir: "",
      fullAuto: false,
    };
    setRoster(defaults);
    saveRoster(defaults);
    setCurrentProjectPath(null);
    markClean();
  }, [providers, markClean]);

  const writeProject = useCallback(async (path: string) => {
    const json = JSON.stringify({ version: 1, roster }, null, 2);
    await writeTextFile(path, json);
    setCurrentProjectPath(path);
    markClean();
  }, [roster, markClean]);

  const handleSaveProject = useCallback(async () => {
    if (currentProjectPath) {
      await writeProject(currentProjectPath).catch(err => console.error("Save project failed:", err));
    } else {
      const path = await loopProjectPickSave().catch(() => null);
      if (!path) return;
      await writeProject(path).catch(err => console.error("Save project failed:", err));
    }
  }, [currentProjectPath, writeProject]);

  const handleSaveAsProject = useCallback(async () => {
    const path = await loopProjectPickSave().catch(() => null);
    if (!path) return;
    await writeProject(path).catch(err => console.error("Save As project failed:", err));
  }, [writeProject]);

  const handleLoadProject = useCallback(async () => {
    const path = await loopProjectPickOpen().catch(() => null);
    if (!path) return;
    try {
      const { content } = await readFile(path);
      const parsed = JSON.parse(content) as { version: number; roster: RosterState };
      if (parsed.roster) {
        setRoster(parsed.roster);
        saveRoster(parsed.roster);
        setCurrentProjectPath(path);
        markClean();
      }
    } catch (err) {
      console.error("Load project failed:", err);
    }
  }, [markClean]);

  const handleResume = useCallback(async (sessionId: string) => {
    try {
      const data = await loopSessionLoad(sessionId);
      const snap = parseLoopSessionData(data);
      const cfg = snap.config;
      updateRoster({
        goal: cfg.goal,
        stoppingCondition: cfg.stoppingCondition,
        orchestratorProvider: cfg.orchestrator.providerId,
        verifierProvider: cfg.verifier.providerId,
        orchestratorName: cfg.orchestrator.name,
        verifierName: cfg.verifier.name,
        subAgents: cfg.subAgents,
        maxLoops: cfg.maxLoops,
        maxOrchestratorSteps: cfg.maxOrchestratorSteps,
        maxInnerIterations: cfg.maxInnerIterations,
      });
      await loop.resume(sessionId);
    } catch (err) {
      console.error("Resume failed:", err);
    }
  }, [loop, updateRoster]);

  const handleCloseConfirm = (canClose: boolean) => {
    setShowCloseConfirm(false);
    closeResolveRef.current?.(canClose);
    closeResolveRef.current = null;
  };

  return (
    <div className="ls-root">
      {showCloseConfirm && (
        <div className="ls-close-overlay">
          <div className="ls-close-dialog">
            <h3 className="ls-close-dialog-title">
              {loop.isRunning ? t.ls_close_title_running : t.ls_close_title_dirty}
            </h3>
            <p className="ls-close-dialog-body">
              {loop.isRunning ? (
                <>{t.ls_close_body_running}</>
              ) : (
                <>
                  {t.ls_close_body_dirty}<br />
                  {currentProjectPath
                    ? t.ls_close_body_modified
                    : t.ls_close_body_unsaved}
                </>
              )}
            </p>
            <div className="ls-close-dialog-actions">
              <button
                type="button"
                className="ls-close-cancel-btn"
                onClick={() => handleCloseConfirm(false)}
              >
                {t.ls_cancel_continue}
              </button>
              <button
                type="button"
                className="ls-close-discard-btn"
                onClick={() => handleCloseConfirm(true)}
              >
                {t.ls_close_discard}
              </button>
            </div>
          </div>
        </div>
      )}
      <div className="ls-left">
        <div className="ls-header">
          <div className="ls-header-top">
            <div>
              <div className="ls-title-row">
                <h2 className="ls-title">Loop Studio</h2>
                {isDirty && <span className="ls-unsaved-dot" title={t.ls_unsaved_tooltip}>●</span>}
              </div>
              <span className="ls-subtitle">{t.ls_subtitle}</span>
            </div>
            <div className="ls-project-toolbar">
              <button
                type="button"
                className="ls-project-btn"
                onClick={handleNewProject}
                disabled={loop.isRunning}
                title={t.ls_clear_project_tooltip}
              >
                🗒 新建
              </button>
              <button
                type="button"
                className="ls-project-btn"
                onClick={handleSaveProject}
                disabled={loop.isRunning}
                title={currentProjectPath ? t.ls_save_tooltip(currentProjectPath) : t.ls_save_default_tooltip}
              >
                💾 儲存
              </button>
              <button
                type="button"
                className="ls-project-btn"
                onClick={handleSaveAsProject}
                disabled={loop.isRunning}
                title={t.ls_save_as_tooltip}
              >
                📋 另存
              </button>
              <button
                type="button"
                className="ls-project-btn"
                onClick={handleLoadProject}
                disabled={loop.isRunning}
                title={t.ls_load_tooltip}
              >
                📂 載入
              </button>
            </div>
          </div>
          {currentProjectPath && (
            <div className="ls-current-file" title={currentProjectPath}>
              {currentProjectPath.split(/[\\/]/).pop()}
            </div>
          )}
        </div>

        <SessionPicker onResume={handleResume} isRunning={loop.isRunning} />

        <div className="ls-section">
          <div className="ls-field">
            <span className="ls-field-label">{t.ls_field_project_dir}</span>
            <div className="ls-dir-row">
              <input
                className="ls-dir-input"
                value={roster.projectDir}
                onChange={e => updateRoster({ projectDir: e.target.value })}
                placeholder={t.ls_dir_placeholder}
                disabled={loop.isRunning}
                spellCheck={false}
              />
              <button
                type="button"
                className="ls-dir-pick-btn"
                onClick={handlePickFolder}
                disabled={loop.isRunning}
                title={t.ls_browse_folder_tooltip}
              >
                📁
              </button>
              {roster.projectDir && (
                <button
                  type="button"
                  className="ls-dir-clear-btn"
                  onClick={() => updateRoster({ projectDir: "" })}
                  disabled={loop.isRunning}
                  title={t.ls_clear_dir_tooltip}
                >
                  ×
                </button>
              )}
            </div>
            {roster.projectDir && (
              <span className="ls-dir-active">{t.ls_dir_active}</span>
            )}
          </div>
        </div>

        <div className="ls-section">
          <div className="ls-field">
            <span className="ls-field-label">{t.ls_field_goal}</span>
            <textarea
              className="ls-goal-input"
              value={roster.goal}
              onChange={e => { updateRoster({ goal: e.target.value }); setPrevText(p => ({ ...p, goal: undefined })); }}
              placeholder={t.ls_goal_placeholder}
              rows={3}
              disabled={loop.isRunning || enhancingField === "goal"}
            />
            <div className="ls-enhance-row">
              <button
                type="button"
                className="ls-enhance-btn"
                onClick={() => handleEnhance("goal")}
                disabled={!roster.goal.trim() || loop.isRunning || enhancingField !== null}
              >
                {enhancingField === "goal" ? t.ls_enhancing_btn : t.ls_enhance_btn}
              </button>
              {prevText.goal && (
                <button
                  type="button"
                  className="ls-undo-btn"
                  onClick={() => handleUndo("goal")}
                >
                  {t.ls_enhance_undo}
                </button>
              )}
            </div>
          </div>
          <div className="ls-field">
            <span className="ls-field-label">{t.ls_field_stopping}</span>
            <textarea
              className="ls-goal-input"
              value={roster.stoppingCondition}
              onChange={e => { updateRoster({ stoppingCondition: e.target.value }); setPrevText(p => ({ ...p, stopping: undefined })); }}
              placeholder={t.ls_stopping_placeholder}
              rows={2}
              disabled={loop.isRunning || enhancingField === "stopping"}
            />
            <div className="ls-enhance-row">
              <button
                type="button"
                className="ls-enhance-btn"
                onClick={() => handleEnhance("stopping")}
                disabled={!roster.stoppingCondition.trim() || loop.isRunning || enhancingField !== null}
              >
                {enhancingField === "stopping" ? t.ls_enhancing_btn : t.ls_enhance_btn}
              </button>
              {prevText.stopping && (
                <button
                  type="button"
                  className="ls-undo-btn"
                  onClick={() => handleUndo("stopping")}
                >
                  {t.ls_enhance_undo}
                </button>
              )}
            </div>
          </div>
        </div>

        <div className="ls-section">
          <span className="ls-section-label">Orchestrator & Verifier</span>
          <div className="ls-ov-row">
            <div className="ls-ov-field">
              <label className="ls-field">
                <span>{t.ls_field_orch_name}</span>
                <input
                  value={roster.orchestratorName}
                  onChange={e => updateRoster({ orchestratorName: e.target.value })}
                  disabled={loop.isRunning}
                />
              </label>
              <label className="ls-field">
                <span>Orchestrator Provider</span>
                <select
                  value={roster.orchestratorProvider}
                  onChange={e => updateRoster({ orchestratorProvider: e.target.value })}
                  disabled={loop.isRunning}
                >
                  {providers.map(p => <option key={p.id} value={p.id}>{p.display_name}</option>)}
                </select>
              </label>
            </div>
            <div className="ls-ov-field">
              <label className="ls-field">
                <span>{t.ls_field_verifier_name}</span>
                <input
                  value={roster.verifierName}
                  onChange={e => updateRoster({ verifierName: e.target.value })}
                  disabled={loop.isRunning}
                />
              </label>
              <label className="ls-field">
                <span>Verifier Provider</span>
                <select
                  value={roster.verifierProvider}
                  onChange={e => updateRoster({ verifierProvider: e.target.value })}
                  disabled={loop.isRunning}
                >
                  {providers.map(p => <option key={p.id} value={p.id}>{p.display_name}</option>)}
                </select>
              </label>
            </div>
          </div>
        </div>

        <AgentRoster
          agents={roster.subAgents}
          providers={providers}
          onChange={subAgents => updateRoster({ subAgents })}
        />

        <div className="ls-section">
          <span className="ls-section-label">{t.ls_section_limits}</span>
          <div className="ls-limits-grid">
            <label className="ls-field">
              <span className="ls-field-label">{t.ls_field_max_loops} <span className="ls-hint-inline">{t.ls_hint_verifier_evals}</span></span>
              <input
                type="number"
                min={1}
                max={999}
                value={roster.maxLoops}
                onChange={e => updateRoster({ maxLoops: Math.max(1, Number(e.target.value)) })}
                className="ls-limit-input"
                disabled={loop.isRunning}
              />
            </label>
            <label className="ls-field">
              <span className="ls-field-label">{t.ls_field_orch_steps} <span className="ls-hint-inline">{t.ls_hint_zero_unlimited}</span></span>
              <input
                type="number"
                min={0}
                value={roster.maxOrchestratorSteps}
                onChange={e => updateRoster({ maxOrchestratorSteps: Math.max(0, Number(e.target.value)) })}
                className="ls-limit-input"
                disabled={loop.isRunning}
              />
            </label>
            <label className="ls-field">
              <span className="ls-field-label">{t.ls_field_agent_tools} <span className="ls-hint-inline">{t.ls_hint_zero_unlimited}</span></span>
              <input
                type="number"
                min={0}
                value={roster.maxInnerIterations}
                onChange={e => updateRoster({ maxInnerIterations: Math.max(0, Number(e.target.value)) })}
                className="ls-limit-input"
                disabled={loop.isRunning}
              />
            </label>
          </div>
          <label className="ls-field ls-fullauto-row">
            <input
              type="checkbox"
              checked={roster.fullAuto ?? false}
              onChange={e => updateRoster({ fullAuto: e.target.checked })}
              disabled={loop.isRunning}
            />
            <span className="ls-field-label">
              {t.ls_full_auto_mode}
              <span className="ls-hint-inline">{t.ls_hint_skip_dangerous}</span>
            </span>
          </label>
        </div>

        {/* Validation issues */}
        {!loop.isRunning && validationIssues.length > 0 && (
          <div className="ls-validation">
            {errors.length > 0 && (
              <div className="ls-validation-group">
                {errors.map((issue, i) => (
                  <div key={i} className="ls-validation-item error">
                    <span className="ls-validation-icon">✕</span>
                    {issue.message}
                  </div>
                ))}
              </div>
            )}
            {warnings.length > 0 && (
              <div className="ls-validation-group">
                {warnings.map((issue, i) => (
                  <div key={i} className="ls-validation-item warning">
                    <span className="ls-validation-icon">⚠</span>
                    {issue.message}
                  </div>
                ))}
                {!warningsDismissed && (
                  <button
                    type="button"
                    className="ls-dismiss-warnings"
                    onClick={() => setWarningsDismissed(true)}
                  >
                    {t.ls_acknowledge_btn}
                  </button>
                )}
              </div>
            )}
          </div>
        )}

        <div className="ls-actions">
          {!loop.isRunning ? (
            <button
              type="button"
              className="ls-start-btn"
              onClick={handleStart}
              disabled={!roster.goal.trim() || !ptySessionId || hasErrors || (hasWarnings && !warningsDismissed)}
            >
              {t.ls_start_loop}
            </button>
          ) : (
            <button
              type="button"
              className="ls-stop-btn"
              onClick={loop.stop}
            >
              {t.ls_stop_loop}
            </button>
          )}
        </div>
      </div>

      <div className="ls-right">
        <div className="ls-trace-header">
          <span className="ls-section-label">{t.ls_section_trace}</span>
          {loop.isRunning && <span className="ls-running-badge">Loop #{loop.iteration}</span>}
        </div>
        {loop.pendingConfirmation && (
          <div className="ls-confirm-panel">
            <div className="ls-confirm-title">
              {t.ls_confirm_dangerous(loop.pendingConfirmation.agentName)}
            </div>
            <pre className="ls-confirm-command">{loop.pendingConfirmation.command}</pre>
            <div className="ls-confirm-actions">
              <button
                type="button"
                className="ls-confirm-deny"
                onClick={() => loop.pendingConfirmation?.resolve(false)}
              >
                {t.ls_confirm_deny}
              </button>
              <button
                type="button"
                className="ls-confirm-allow"
                onClick={() => loop.pendingConfirmation?.resolve(true)}
              >
                {t.ls_confirm_allow}
              </button>
            </div>
          </div>
        )}
        <ExecutionTrace trace={loop.trace} isRunning={loop.isRunning} iteration={loop.iteration} timingMode={timingMode} />
      </div>
    </div>
  );
}
