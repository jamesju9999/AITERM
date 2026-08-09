import { Fragment, useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { Translations } from "../../lib/i18n";
import { getConfig } from "../../ipc/config";
import { listProviders } from "../../ipc/provider";
import type { ProviderInfo } from "../../ipc/provider";
import {
  bridgeSetConfig,
  bridgeStatus,
  type BridgeStatus,
  type ClaudeBridgeConfig,
  type TierMapping,
} from "../../ipc/bridge";
import "./ClaudeBridgePage.css";

/** 與 `useTerminalBlocks.ts` 的判斷方式一致，避免兩處用不同的偵測。 */
const isWindows = navigator.platform.toLowerCase().startsWith("win");

/**
 * 目前支援的 provider type，與 src-tauri/src/bridge/factory.rs 的 kind_for
 * 逐項對齊：openai 系（含 openai/openai-compatible/ollama/openrouter/
 * deepseek/kimi/xai/github-copilot）、anthropic 系（anthropic/
 * anthropic-compatible）與 codex（M2）皆支援。
 */
const SUPPORTED_TYPES = new Set([
  "openai",
  "openai-compatible",
  "ollama",
  "openrouter",
  "deepseek",
  "kimi",
  "xai",
  "github-copilot",
  "anthropic",
  "anthropic-compatible",
  "codex",
]);

function isSupported(p: ProviderInfo): boolean {
  // google-ai 兩種模式現在都支援：oauth 走 Antigravity（M3），API key 模式
  // 等同 OpenAI 相容端點。
  if (p.provider_type === "google-ai") return true;
  return SUPPORTED_TYPES.has(p.provider_type);
}

const TIERS = ["opus", "sonnet", "haiku"] as const;
type TierKey = (typeof TIERS)[number];

// t 是資料物件（translations[locale]），不是函式，所以層級標籤不能用樣板
// 字串組 key 動態查（那樣 TS 會把回傳型別推成所有 key 的聯集，含函式型
// 的字串），逐一列分支才能保留字面 key 對應的 string 型別。
function tierLabel(t: Translations, tier: TierKey): string {
  switch (tier) {
    case "opus":
      return t.bridge_tier_opus;
    case "sonnet":
      return t.bridge_tier_sonnet;
    case "haiku":
      return t.bridge_tier_haiku;
  }
}

function tierHint(t: Translations, tier: TierKey): string {
  switch (tier) {
    case "opus":
      return t.bridge_tier_opus_hint;
    case "sonnet":
      return t.bridge_tier_sonnet_hint;
    case "haiku":
      return t.bridge_tier_haiku_hint;
  }
}

export function ClaudeBridgePage() {
  const { t } = useLocale();
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [cfg, setCfg] = useState<ClaudeBridgeConfig | null>(null);
  const [status, setStatus] = useState<BridgeStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void (async () => {
      const [c, ps, s] = await Promise.all([getConfig(), listProviders(), bridgeStatus()]);
      setProviders(ps);
      setCfg(c.claude_bridge);
      setStatus(s);
    })();
  }, []);

  /**
   * 所有對設定的改動都走這裡，順便撤掉「已儲存」——不然按鈕會對著已經不同的
   * 內容繼續宣稱存過了。放在改動處而不是 cfg 的 useEffect 裡：effect 裡同步
   * setState 會觸發連鎖 render（eslint react-hooks/set-state-in-effect），
   * 而且初次載入設定時也會多跑一次。
   */
  const updateCfg: typeof setCfg = useCallback((next) => {
    setSaved(false);
    setCfg(next);
  }, []);

  const setTier = useCallback(
    (tier: TierKey, providerId: string) => {
      updateCfg((prev) => {
        if (!prev) return prev;
        if (!providerId) return { ...prev, [tier]: null };
        const p = providers.find((x) => x.id === providerId);
        const mapping: TierMapping = { provider_id: providerId, model: p?.model ?? "" };
        return { ...prev, [tier]: mapping };
      });
    },
    [providers, updateCfg],
  );

  const setTierModel = useCallback((tier: TierKey, model: string) => {
    updateCfg((prev) => {
      const current = prev?.[tier];
      if (!prev || !current) return prev;
      return { ...prev, [tier]: { ...current, model } };
    });
  }, [updateCfg]);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    try {
      setStatus(await bridgeSetConfig(cfg));
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }, [cfg]);


  /**
   * 產生可直接貼進終端機的手動啟動命令。
   *
   * 必須分平台：PowerShell 不認得 POSIX 的 `VAR=value cmd` 前綴語法，會把
   * `ANTHROPIC_BASE_URL=...` 當成指令名稱並回報「is not recognized as a name
   * of a cmdlet」——這是使用者在 Windows 上實際撞到的。
   *
   * 環境變數清單與 `src-tauri/src/bridge/env.rs` 的 `bridge_envs` /
   * `ENV_TO_REMOVE` 對應：分頁自動注入的與這裡手動貼的必須一致，否則兩條
   * 路徑的行為會分歧。
   */
  const manualCommand = (): string => {
    const port = status?.port ?? cfg?.port ?? 8317;
    const token = status?.token ?? "<token>";
    const vars: [string, string][] = [
      // 不能帶 /v1 後綴，Claude Code 自己會接上 /v1/messages。
      ["ANTHROPIC_BASE_URL", `http://127.0.0.1:${port}`],
      ["ANTHROPIC_AUTH_TOKEN", token],
      ["ANTHROPIC_DEFAULT_OPUS_MODEL", "aiterm:opus"],
      ["ANTHROPIC_DEFAULT_SONNET_MODEL", "aiterm:sonnet"],
      ["ANTHROPIC_DEFAULT_HAIKU_MODEL", "aiterm:haiku"],
      ["API_TIMEOUT_MS", "3000000"],
      ["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"],
    ];
    if (isWindows) {
      return [
        ...vars.map(([k, v]) => `$env:${k} = "${v}"`),
        // 使用者環境本來就有的 API key 是難查的干擾源（症狀是「設了橋接卻
        // 打到真的 Anthropic」），跟分頁注入時的 env_removals 一致。
        `Remove-Item Env:ANTHROPIC_API_KEY -ErrorAction SilentlyContinue`,
        `claude`,
      ].join("\n");
    }
    return [...vars.map(([k, v]) => `${k}='${v}'`), `ANTHROPIC_API_KEY=`, `claude`].join(" ");
  };

  if (!cfg) return <div className="bridge-page" />;

  return (
    <div className="bridge-page">
      <h2>{t.bridge_title}</h2>
      <p className="bridge-desc">{t.bridge_desc}</p>

      <div className="bridge-status">
        <span className={status?.running ? "bridge-dot bridge-dot--on" : "bridge-dot"} />
        {status?.running ? t.bridge_status_running : t.bridge_status_stopped}
        {status?.port ? ` · :${status.port}` : ""}
      </div>
      {status?.error && <div className="bridge-error">{status.error}</div>}

      <section className="bridge-section">
        <h3>{t.bridge_section_server}</h3>

        <label className="bridge-row">
          <input
            type="checkbox"
            checked={cfg.enabled}
            onChange={(e) => updateCfg({ ...cfg, enabled: e.target.checked })}
          />
          {t.bridge_enable}
        </label>

        <label className="bridge-row">
          {t.bridge_port}
          <input
            type="number"
            value={cfg.port}
            onChange={(e) => updateCfg({ ...cfg, port: Number(e.target.value) || 8317 })}
          />
        </label>

        <label className="bridge-row">
          <input
            type="checkbox"
            checked={cfg.default_on_new_tab}
            onChange={(e) => updateCfg({ ...cfg, default_on_new_tab: e.target.checked })}
          />
          {t.bridge_default_on_new_tab}
        </label>
      </section>

      <section className="bridge-section">
        <h3>{t.bridge_section_tiers}</h3>
        <p className="bridge-section-desc">{t.bridge_section_tiers_desc}</p>

        {/* grid 而不是每列各自 flex：三欄要對齊，而且模型欄得能跟著面板寬度縮，
            原本 label 220px + select 200px + 不會縮的 input 加起來撐破面板，
            整頁被推出一條水平捲軸、模型 ID 尾巴看不到。 */}
        <div className="bridge-tier-grid">
          <span className="bridge-tier-head">{t.bridge_tier_column}</span>
          <span className="bridge-tier-head">{t.bridge_tier_provider}</span>
          <span className="bridge-tier-head">{t.bridge_tier_model}</span>

          {TIERS.map((tier) => {
            const label = tierLabel(t, tier);
            return (
              <Fragment key={tier}>
                <label className="bridge-tier-name" htmlFor={`bridge-${tier}`}>
                  {label}
                  <span className="bridge-tier-hint">{tierHint(t, tier)}</span>
                </label>
                <select
                  id={`bridge-${tier}`}
                  value={cfg[tier]?.provider_id ?? ""}
                  onChange={(e) => setTier(tier, e.target.value)}
                >
                  <option value="">{t.bridge_tier_unset}</option>
                  {providers.map((p) => (
                    <option key={p.id} value={p.id} disabled={!isSupported(p)}>
                      {p.display_name}
                      {isSupported(p) ? "" : t.bridge_unsupported_suffix}
                    </option>
                  ))}
                </select>
                {cfg[tier] ? (
                  <input
                    aria-label={`${label} ${t.bridge_tier_model}`}
                    value={cfg[tier]!.model}
                    onChange={(e) => setTierModel(tier, e.target.value)}
                  />
                ) : (
                  // 佔住格子，否則沒設供應商的那一列會讓下一列往上遞補，整個表格錯位。
                  <span />
                )}
              </Fragment>
            );
          })}
        </div>
      </section>

      <div className="bridge-actions">
        <button onClick={() => void save()} disabled={saving}>
          {saved ? `${t.bridge_saved} ✓` : t.save}
        </button>
        <button
          onClick={() => {
            void navigator.clipboard.writeText(manualCommand());
            setCopied(true);
          }}
        >
          {copied ? t.bridge_copied : t.bridge_copy_command}
        </button>
      </div>
    </div>
  );
}
