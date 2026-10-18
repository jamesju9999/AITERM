import { useEffect, useRef, useState } from "react";
import { confirm, open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { checkRepo, initRepo, type AgentCli, type AgentKind, type AgentRow, type RepoCheck } from "../../ipc/agents";
import { codexInfo, type CodexInfo } from "../../ipc/tasks";
import { usedDirs } from "../../ipc/projects";
import { deleteCustomRole, loadCustomRoles, saveCustomRole, type CustomRole } from "./customRoles";
import { builtinRoles, type RolePreset } from "./roles";

export interface AgentConfigValues {
  name: string;
  role: string;
  prompt_prefix: string;
  project_dir: string;
  /** 只對 Coder 有意義。 */
  produces_spec: boolean;
  /** 派工時的 `--model`；空字串 = 沿用預設。 */
  model: string;
  /** 用哪個 AI 工具做事。 */
  cli: AgentCli;
}

/** 模型欄位的建議值（Claude Code 的別名）；也可以輸入完整的模型名稱。 */
const MODEL_SUGGESTIONS = ["opus", "sonnet", "haiku", "opusplan"];

/**
 * 「指派角色」對話框。新建時才能選程式庫；編輯時程式庫鎖住——Agent 的
 * 分支已經從那個 repo 分出去了。送出失敗（例如不是 git repo）的錯誤顯示
 * 在對話框裡，對話框不關，使用者改完可以直接再送。
 */
export function AgentConfigDialog({
  projectId,
  kind,
  agent,
  defaultName,
  onCancel,
  onSubmit,
}: {
  projectId: string;
  kind: AgentKind;
  /** `null` = 新建。 */
  agent: AgentRow | null;
  defaultName: string;
  onCancel: () => void;
  onSubmit: (values: AgentConfigValues) => Promise<void>;
}) {
  const { t } = useLocale();
  const isNew = agent === null;
  const roles = builtinRoles(kind, t);
  const [mine, setMine] = useState<CustomRole[]>(() => loadCustomRoles().filter((r) => r.kind === kind));
  const [menuOpen, setMenuOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [name, setName] = useState(agent?.name ?? defaultName);
  const [role, setRole] = useState(agent?.role ?? roles[0].name);
  const [prefix, setPrefix] = useState(agent?.prompt_prefix ?? roles[0].prefix);
  const [dir, setDir] = useState(agent?.project_dir ?? "");
  const [producesSpec, setProducesSpec] = useState(agent?.produces_spec ?? false);
  // 新的審核者、驗收者預設用 Sonnet：判定與照條件驗收不需要最貴的模型，可以改。
  const [model, setModel] = useState(agent?.model ?? (isNew && (kind === "reviewer" || kind === "verifier") ? "sonnet" : ""));
  const [cli, setCli] = useState<AgentCli>(agent?.cli ?? "claude");
  // Codex 一律走 OpenAI；使用者 config.toml 的預設模型不一定是 ChatGPT 帳號能用的，所以必填。
  const modelMissing = cli === "codex" && !model.trim();
  const [codex, setCodex] = useState<CodexInfo | null>(null);
  useEffect(() => {
    if (cli !== "codex" || codex) return;
    let alive = true;
    codexInfo()
      .then((info) => {
        if (!alive) return;
        setCodex(info);
        // 還沒填模型就先選清單第一個（Codex 自己的預設）。
        if (info.models.length > 0) setModel((m) => (m.trim() ? m : info.models[0].slug));
      })
      .catch((e) => alive && setCodex({ models: [], login: null, error: String(e) }));
    return () => {
      alive = false;
    };
  }, [cli, codex]);
  const pickCli = (next: AgentCli) => {
    setCli(next);
    // Claude 的別名（sonnet…）對 Codex 沒有意義，切過去時清掉。
    if (next === "codex" && MODEL_SUGGESTIONS.includes(model.trim())) setModel("");
  };
  const [dirs, setDirs] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  /** 程式庫檢查結果。null = 還沒查完（或欄位是空的）。 */
  const [repo, setRepo] = useState<RepoCheck | null>(null);
  const [repoBusy, setRepoBusy] = useState(false);
  const [repoRev, setRepoRev] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isNew) return;
    let alive = true;
    usedDirs(projectId)
      .then((d) => {
        if (!alive) return;
        setDirs(d);
        setDir((cur) => cur || d[0] || "");
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [isNew, projectId]);

  // 選／輸入資料夾就檢查（打字時等 300ms 再查），不讓問題拖到按「放上桌布」才出現。
  useEffect(() => {
    if (!isNew) return;
    const path = dir.trim();
    if (!path) return;
    let alive = true;
    const h = window.setTimeout(() => {
      checkRepo(path)
        .then((r) => alive && setRepo(r))
        .catch(() => alive && setRepo(null));
    }, 300);
    return () => {
      alive = false;
      window.clearTimeout(h);
    };
  }, [isNew, dir, repoRev]);

  const fixRepo = async () => {
    const path = dir.trim();
    const msg = repo?.status === "no_commits" ? t.ac_repo_first_commit_confirm(path) : t.ac_repo_init_confirm(path);
    if (!(await confirm(msg, { kind: "info" }))) return;
    setRepoBusy(true);
    setError(null);
    try {
      await initRepo(path, projectId);
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    }
    setRepoBusy(false);
    setRepoRev((n) => n + 1);
  };

  const repoReady = !isNew || repo?.status === "ok";

  const pickRole = (r: Pick<RolePreset, "name" | "prefix">) => {
    setRole(r.name);
    setPrefix(r.prefix);
    // 選「規格撰寫師」就順手勾起產出規格；選其他內建角色就取消。我的角色不動。
    if (kind === "coder" && roles.some((b) => b.name === r.name)) setProducesSpec(r.name === t.ac_role_spec);
    setMenuOpen(false);
    setNotice(null);
  };

  // 點選單外面就關掉。
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  const saveMine = () => {
    const n = role.trim();
    if (!n) return;
    setMine(saveCustomRole({ kind, name: n, prefix }).filter((r) => r.kind === kind));
    setNotice(t.ac_role_saved(n));
  };

  const removeMine = async (r: CustomRole) => {
    // window.confirm 在 Tauri webview 裡沒有實作，要用 plugin-dialog。
    if (!(await confirm(t.ac_role_delete_confirm(r.name), { kind: "warning" }))) return;
    setMine(deleteCustomRole(r.id).filter((x) => x.kind === kind));
  };

  const browse = async () => {
    try {
      const picked = await open({ directory: true, defaultPath: dir || undefined });
      if (typeof picked === "string") setDir(picked);
    } catch (e) {
      setError(String(e));
    }
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await onSubmit({
        name: name.trim() || defaultName,
        role: role.trim(),
        prompt_prefix: prefix,
        project_dir: dir.trim(),
        produces_spec: kind === "coder" && producesSpec,
        model: model.trim(),
        cli,
      });
    } catch (e) {
      // Tauri 的 invoke 錯誤是字串；其他情況才是 Error。
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
      setBusy(false);
    }
  };

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog" role="dialog" aria-label={isNew ? t.ac_dialog_new_title : t.ac_dialog_edit_title}>
        <h3 className="task-dialog-title">{isNew ? t.ac_dialog_new_title : t.ac_dialog_edit_title}</h3>
        <p className="ac-dialog-desc">{t.ac_dialog_desc}</p>
        <label className="task-field">
          <span className="task-field-label">{t.ac_name}</span>
          <input className="task-field-input" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        </label>
        <div className="task-field">
          <div className="ac-role-head">
            <span className="task-field-label">{t.ac_role_name}</span>
            <div className="ac-role-menu-wrap" ref={menuRef}>
              <button
                type="button"
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                onClick={() => setMenuOpen((v) => !v)}
              >
                {t.ac_role_presets} ▾
              </button>
              {menuOpen && (
                <div className="ac-role-menu" role="menu">
                  <div className="ac-role-menu-label">{t.ac_role_builtin}</div>
                  {roles.map((r) => (
                    <button key={r.name} type="button" role="menuitem" className="ac-role-menu-item" onClick={() => pickRole(r)}>
                      <span aria-hidden>{r.emoji}</span> {r.name}
                    </button>
                  ))}
                  {mine.length > 0 && <div className="ac-role-menu-label">{t.ac_role_mine}</div>}
                  {mine.map((r) => (
                    <div key={r.id} className="ac-role-menu-row">
                      <button type="button" role="menuitem" className="ac-role-menu-item" onClick={() => pickRole(r)}>
                        <span aria-hidden>⭐</span> {r.name}
                      </button>
                      <button
                        type="button"
                        className="ac-iconbtn"
                        title={t.ac_delete}
                        aria-label={`${t.ac_delete} ${r.name}`}
                        onClick={() => void removeMine(r)}
                      >
                        ×
                      </button>
                    </div>
                  ))}
                  <div className="ac-role-menu-sep" />
                  <button
                    type="button"
                    role="menuitem"
                    className="ac-role-menu-item ac-role-menu-item--muted"
                    onClick={() => pickRole({ name: "", prefix: "" })}
                  >
                    <span aria-hidden>✏️</span> {t.ac_role_custom_blank}
                  </button>
                </div>
              )}
            </div>
          </div>
          <input
            className="task-field-input"
            value={role}
            onChange={(e) => {
              setRole(e.target.value);
              setNotice(null);
            }}
            aria-label={t.ac_role_name}
          />
        </div>
        <label className="task-field">
          <span className="task-field-label">{t.ac_prefix}</span>
          <textarea
            className="task-field-input task-field-textarea"
            value={prefix}
            onChange={(e) => {
              setPrefix(e.target.value);
              setNotice(null);
            }}
            rows={5}
          />
        </label>
        {kind === "coder" && (
          <label className="ac-produces-spec">
            <input type="checkbox" checked={producesSpec} onChange={(e) => setProducesSpec(e.target.checked)} />
            <span>📐 {t.ac_produces_spec}</span>
          </label>
        )}
        <div className="task-field">
          <span className="task-field-label">{t.ac_cli}</span>
          <div className="ac-cli" role="radiogroup" aria-label={t.ac_cli}>
            {(["claude", "codex"] as const).map((c) => (
              <label key={c} className={`ac-cli-option${cli === c ? " ac-cli-option--on" : ""}`}>
                <input type="radio" name="ac-cli" checked={cli === c} onChange={() => pickCli(c)} />
                {c === "claude" ? "Claude Code" : "Codex"}
              </label>
            ))}
          </div>
        </div>
        <label className="task-field">
          <span className="task-field-label">{t.ac_model}</span>
          <input
            className="task-field-input"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            placeholder={cli === "codex" ? t.ac_model_placeholder_codex : t.ac_model_placeholder}
            list={cli === "codex" ? "ac-codex-models" : "ac-model-suggestions"}
            aria-label={t.ac_model}
            spellCheck={false}
          />
          <datalist id="ac-model-suggestions">
            {MODEL_SUGGESTIONS.map((m) => (
              <option key={m} value={m} />
            ))}
          </datalist>
          <datalist id="ac-codex-models">
            {(codex?.models ?? []).map((m) => (
              <option key={m.slug} value={m.slug}>
                {m.name}
              </option>
            ))}
          </datalist>
          <span className={`task-field-hint${modelMissing ? " ac-bad-text" : ""}`}>
            {cli === "codex" ? t.ac_model_hint_codex : t.ac_model_hint}
          </span>
          {cli === "codex" && codex?.login && (
            <span className="task-field-hint">
              {t.ac_codex_login}
              {codex.login}
            </span>
          )}
          {cli === "codex" && codex?.error && (
            <span className="task-field-hint ac-bad-text">
              {t.ac_codex_info_error}
              {codex.error}
            </span>
          )}
        </label>
        <div className="ac-role-save">
          <button
            type="button"
            className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
            disabled={!role.trim() || !prefix.trim()}
            onClick={saveMine}
          >
            ⭐ {t.ac_role_save}
          </button>
          {notice && <span className="ac-role-notice">{notice}</span>}
        </div>
        <div className="task-field">
          <span className="task-field-label">{t.ac_repo}</span>
          {isNew ? (
            <>
              <div className="task-field-row">
                <input
                  className="task-field-input task-field-input--mono"
                  value={dir}
                  onChange={(e) => {
                    setDir(e.target.value);
                    setRepo(null);
                  }}
                  aria-label={t.ac_repo}
                />
                <button type="button" className="aiterm-btn aiterm-btn--secondary" onClick={() => void browse()}>
                  {t.ac_browse}
                </button>
              </div>
              {dirs.length > 1 && (
                <div className="ac-roles">
                  {dirs.map((d) => (
                    <button
                      key={d}
                      type="button"
                      className={`ac-role ac-role--mono${d === dir ? " ac-role--on" : ""}`}
                      onClick={() => {
                        setDir(d);
                        setRepo(null);
                      }}
                    >
                      {d}
                    </button>
                  ))}
                </div>
              )}
              {dir.trim() && repo && (
                <div className={`ac-repo-status ac-repo-status--${repo.status}`} data-testid="repo-status" role="status">
                  {repo.status === "ok" && <span>✓ {t.ac_repo_ok(repo.branch)}</span>}
                  {repo.status === "missing" && <span>✗ {t.ac_repo_missing}</span>}
                  {(repo.status === "not_repo" || repo.status === "no_commits") && (
                    <>
                      <span>⚠ {repo.status === "not_repo" ? t.ac_repo_not_repo : t.ac_repo_no_commits}</span>
                      <button
                        type="button"
                        className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                        disabled={repoBusy}
                        onClick={() => void fixRepo()}
                      >
                        {repoBusy ? t.ac_repo_initing : repo.status === "not_repo" ? t.ac_repo_init : t.ac_repo_first_commit}
                      </button>
                    </>
                  )}
                </div>
              )}
            </>
          ) : (
            <div className="ac-readonly" title={t.ac_repo_locked}>
              {agent.project_dir}
              {agent.branch ? ` · ${agent.branch}` : ""}
            </div>
          )}
        </div>
        {error && (
          <p className="task-field-error" role="alert">
            {error}
          </p>
        )}
        <div className="ac-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" disabled={busy} onClick={onCancel}>
            {t.ac_cancel}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" disabled={busy || (isNew && !dir.trim()) || !repoReady || modelMissing} onClick={() => void submit()}>
            {isNew ? t.ac_place : t.ac_save}
          </button>
        </div>
      </div>
    </div>
  );
}
