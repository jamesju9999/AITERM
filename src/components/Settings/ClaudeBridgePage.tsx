import { useCallback, useEffect, useState } from "react";

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
  // google-ai 的 oauth 模式走 Antigravity（M3）；API key 模式等同 OpenAI 相容端點。
  if (p.provider_type === "google-ai") return p.auth_method !== "oauth";
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

export function ClaudeBridgePage() {
  const { t } = useLocale();
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [cfg, setCfg] = useState<ClaudeBridgeConfig | null>(null);
  const [status, setStatus] = useState<BridgeStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    void (async () => {
      const [c, ps, s] = await Promise.all([getConfig(), listProviders(), bridgeStatus()]);
      setProviders(ps);
      setCfg(c.claude_bridge);
      setStatus(s);
    })();
  }, []);

  const setTier = useCallback(
    (tier: TierKey, providerId: string) => {
      setCfg((prev) => {
        if (!prev) return prev;
        if (!providerId) return { ...prev, [tier]: null };
        const p = providers.find((x) => x.id === providerId);
        const mapping: TierMapping = { provider_id: providerId, model: p?.model ?? "" };
        return { ...prev, [tier]: mapping };
      });
    },
    [providers],
  );

  const setTierModel = useCallback((tier: TierKey, model: string) => {
    setCfg((prev) => {
      const current = prev?.[tier];
      if (!prev || !current) return prev;
      return { ...prev, [tier]: { ...current, model } };
    });
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    try {
      setStatus(await bridgeSetConfig(cfg));
    } finally {
      setSaving(false);
    }
  }, [cfg]);

  const manualCommand = (): string => {
    const port = status?.port ?? cfg?.port ?? 8317;
    const token = status?.token ?? "<token>";
    return [
      `ANTHROPIC_BASE_URL='http://127.0.0.1:${port}'`,
      `ANTHROPIC_AUTH_TOKEN='${token}'`,
      `ANTHROPIC_DEFAULT_OPUS_MODEL='aiterm:opus'`,
      `ANTHROPIC_DEFAULT_SONNET_MODEL='aiterm:sonnet'`,
      `ANTHROPIC_DEFAULT_HAIKU_MODEL='aiterm:haiku'`,
      `API_TIMEOUT_MS=3000000`,
      `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`,
      `claude`,
    ].join(" ");
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

      <label className="bridge-row">
        <input
          type="checkbox"
          checked={cfg.enabled}
          onChange={(e) => setCfg({ ...cfg, enabled: e.target.checked })}
        />
        {t.bridge_enable}
      </label>

      <label className="bridge-row">
        {t.bridge_port}
        <input
          type="number"
          value={cfg.port}
          onChange={(e) => setCfg({ ...cfg, port: Number(e.target.value) || 8317 })}
        />
      </label>

      <label className="bridge-row">
        <input
          type="checkbox"
          checked={cfg.default_on_new_tab}
          onChange={(e) => setCfg({ ...cfg, default_on_new_tab: e.target.checked })}
        />
        {t.bridge_default_on_new_tab}
      </label>

      {TIERS.map((tier) => {
        const label = tierLabel(t, tier);
        return (
          <div className="bridge-tier" key={tier}>
            <label htmlFor={`bridge-${tier}`}>{label}</label>
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
            {cfg[tier] && (
              <input
                aria-label={`${label} ${t.bridge_tier_model}`}
                value={cfg[tier]!.model}
                onChange={(e) => setTierModel(tier, e.target.value)}
              />
            )}
          </div>
        );
      })}

      <div className="bridge-actions">
        <button onClick={() => void save()} disabled={saving}>
          {t.save}
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
