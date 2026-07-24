# AI Provider Form UI Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure `ProviderForm.tsx` into three visually grouped sections (基本資訊 / 驗證方式 / 端點與模型), fix the cramped GitHub Copilot auth-button bug, and adopt the app's existing `aiterm-btn` button system and CSS color tokens instead of the form's bespoke blue styling — with zero changes to business logic, IPC calls, or state management.

**Architecture:** Pure presentation-layer change. A new tiny `FormSection` wrapper component groups existing (unmodified) field blocks. CSS changes swap hardcoded colors for existing `var(--accent)` / `var(--bg-secondary)` / `var(--border-color)` tokens and remove now-redundant bespoke button styles in favor of the shared `aiterm-btn` classes already used elsewhere in the app.

**Tech Stack:** React 19 + TypeScript (frontend), CSS custom properties (no CSS framework), i18n via `src/lib/i18n.ts`.

**Spec:** `docs/superpowers/specs/2026-07-24-provider-form-ui-polish-design.md`

---

## Task 1: Add i18n keys for the three section titles

**Files:**
- Modify: `src/lib/i18n.ts:117-121` (zh-TW block), `:1100-1104` (en block)

- [ ] **Step 1: Add the zh-TW keys**

In `src/lib/i18n.ts`, find this block (around line 117-121):

```ts
    // ProviderForm
    edit_provider: "編輯供應商",
    new_provider: "新增供應商",
    provider_type: "類型",
```

Insert three new keys immediately after `new_provider` and before `provider_type`:

```ts
    // ProviderForm
    edit_provider: "編輯供應商",
    new_provider: "新增供應商",
    provider_section_basic: "基本資訊",
    provider_section_auth: "驗證方式",
    provider_section_endpoint: "端點與模型",
    provider_type: "類型",
```

- [ ] **Step 2: Add the English overrides**

In the same file, find the corresponding English block (around line 1100-1104):

```ts
    // ProviderForm
    edit_provider: "Edit Provider",
    new_provider: "New Provider",
    provider_type: "Type",
```

Insert the English overrides in the same position:

```ts
    // ProviderForm
    edit_provider: "Edit Provider",
    new_provider: "New Provider",
    provider_section_basic: "Basic Info",
    provider_section_auth: "Authentication",
    provider_section_endpoint: "Endpoint & Model",
    provider_type: "Type",
```

(`en` is built as `{ ...zhTW, ...enRaw }` — see the bottom of the file — so skipping this step would leave the English UI showing the Chinese section titles. Both edits are required.)

- [ ] **Step 3: Verify**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors.

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(settings): add i18n keys for provider form section titles"
```

---

## Task 2: `ProviderForm.css` — tokens, section styles, button cleanup

**Files:**
- Modify: `src/components/Settings/ProviderForm.css`

- [ ] **Step 1: Remove the shouty-uppercase label styling**

Find:

```css
.form-group label {
  font-size: 12px;
  color: #aaa;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
```

Replace with:

```css
.form-group label {
  font-size: 12px;
  color: #aaa;
}
```

- [ ] **Step 2: Add the section container styles**

Immediately after the `.form-group--checkbox input[type="checkbox"]` rule (right before the existing `.presets` rule), add:

```css
.form-section {
  background: var(--bg-secondary, #141414);
  border: 1px solid var(--border-color, #2a2a2a);
  border-radius: 8px;
  padding: 14px;
  margin-bottom: 12px;
}

.form-section:last-of-type {
  margin-bottom: 16px;
}

.form-section-title {
  color: var(--accent, #a855f7);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.03em;
  margin-bottom: 10px;
}

.form-section .form-group:last-child {
  margin-bottom: 0;
}
```

- [ ] **Step 3: Update preset buttons to use the shared button system**

Find:

```css
.preset-btn {
  background: #2a2a2a;
  border: 1px solid #3a3a3a;
  border-radius: 4px;
  color: #bbb;
  cursor: pointer;
  font-size: 11px;
  padding: 3px 8px;
}

.preset-btn:hover {
  background: #333;
  color: #e6e6e6;
}
```

Delete this whole block — preset buttons now use `aiterm-btn aiterm-btn--secondary aiterm-btn--sm` (added to the JSX in Task 3), which is styled globally by `src/styles/buttons.css`.

- [ ] **Step 4: Remove the now-unused bespoke `.form-actions button` styles**

Find:

```css
.form-actions button {
  background: #2a2a2a;
  border: 1px solid #3a3a3a;
  border-radius: 4px;
  color: #ccc;
  cursor: pointer;
  font-size: 13px;
  padding: 7px 16px;
}

.form-actions button:hover:not(:disabled) {
  background: #333;
}

.form-actions button.btn-primary {
  background: #2d5aab;
  border-color: #3d6ac0;
  color: #fff;
}

.form-actions button.btn-primary:hover:not(:disabled) {
  background: #3265be;
}

.form-actions button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}
```

Delete this whole block. The two buttons in `.form-actions` will carry `aiterm-btn aiterm-btn--secondary` / `aiterm-btn aiterm-btn--primary` classes (Task 3), which already handle color, hover, and `:disabled` styling globally — including `.form-actions` in the selector was only ever needed to scope the old bespoke rules, so nothing needs to replace this block.

Leave the `.form-actions` rule itself (the flex container: `display: flex; gap: 8px; justify-content: flex-end; margin-top: 20px;`) untouched — it's layout, not button coloring.

- [ ] **Step 5: Update the Anthropic auth-tab active state to use the accent gradient**

Find:

```css
.auth-tab.active {
  background: #2d5aab;
  color: #fff;
}
```

Replace with:

```css
.auth-tab.active {
  background: var(--accent-gradient, linear-gradient(135deg, #a855f7, #6366f1));
  color: #fff;
}
```

(Leave `.auth-tab`, `.auth-tab:hover:not(.active)`, `.anthropic-auth-tabs` untouched — only the active-state color is in scope per the design spec.)

- [ ] **Step 6: Remove the now-unused bespoke OAuth button styles**

Find and delete these three blocks in full (their buttons switch to `aiterm-btn` classes in Task 3, so these bespoke rules become dead code):

```css
.anthropic-oauth-open {
  font-size: 12px;
  padding: 5px 14px;
  background: #1e3a6e;
  border: 1px solid #3d6ac0;
  color: #7aadff;
  border-radius: 5px;
  cursor: pointer;
  white-space: nowrap;
}

.anthropic-oauth-open:hover:not(:disabled) {
  background: #2a4a8a;
  color: #fff;
}

.anthropic-oauth-open:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
```

```css
.anthropic-oauth-logout {
  font-size: 11px;
  padding: 3px 10px;
  background: transparent;
  border: 1px solid #444;
  color: #999;
  border-radius: 4px;
  cursor: pointer;
  flex-shrink: 0;
}

.anthropic-oauth-logout:hover:not(:disabled) {
  border-color: #777;
  color: #ddd;
}
```

```css
.anthropic-oauth-cancel-btn {
  font-size: 12px;
  padding: 5px 12px;
  background: transparent;
  border: 1px solid #444;
  color: #888;
  border-radius: 4px;
  cursor: pointer;
}

.anthropic-oauth-cancel-btn:hover {
  border-color: #666;
  color: #bbb;
}
```

Do **not** delete `.anthropic-oauth-done`, `.anthropic-oauth-ok`, `.anthropic-oauth-flow`, `.anthropic-oauth-step`, `.oauth-step-num`, `.anthropic-oauth-code-input`, `.anthropic-oauth-btns`, `.anthropic-oauth-paste-ui`, `.anthropic-oauth-paste-hint`, `.anthropic-oauth-url-input`, `.form-hint--error` — these are layout/status containers, not button color styles, and stay exactly as-is (some, like `.oauth-step-num`/`.anthropic-oauth-step`, appear unused by current JSX — that's pre-existing and out of scope to clean up here).

- [ ] **Step 7: Verify no other file references the deleted classes**

Run: `grep -rn "preset-btn\|btn-primary\|anthropic-oauth-open\|anthropic-oauth-logout\|anthropic-oauth-cancel-btn" src/components/Settings/ProviderForm.tsx`
Expected: no matches — Task 3 removes every JSX reference to these class names in the same commit sequence. If this step runs before Task 3's edits, matches are expected at this point; re-run it after Task 3 to confirm zero matches remain.

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/ProviderForm.css
git commit -m "style(settings): adopt app color tokens and aiterm-btn system in provider form CSS"
```

Note: this commit alone will leave the JSX referencing now-deleted CSS classes (`preset-btn`, `btn-primary`, etc.) until Task 3 lands — that's fine within this same plan/branch, but don't ship Task 2 alone to a shared branch without Task 3.

---

## Task 3: `ProviderForm.tsx` — add `FormSection`, regroup fields, fix the Copilot bug, swap button classNames

**Files:**
- Modify: `src/components/Settings/ProviderForm.tsx`

This is the largest task. It touches the top-level import, adds one small helper component, and replaces the entire JSX `return` statement. No hook, state, effect, or handler logic changes anywhere in this task — only what's rendered and how it's grouped/styled.

- [ ] **Step 1: Add the `ReactNode` type import**

Find the first line of the file:

```tsx
import { useState, useEffect } from "react";
```

Replace with:

```tsx
import { useState, useEffect, type ReactNode } from "react";
```

- [ ] **Step 2: Add the `FormSection` helper component**

Find:

```tsx
const PROVIDER_TYPES: ProviderType[] = [
  "openai",
  "anthropic",
  "ollama",
  "openai-compatible",
  "github-copilot",
  "google-ai",
  "openrouter",
  "xai",
  "deepseek",
  "kimi",
  "anthropic-compatible",
];

export function ProviderForm({ existing, onSave, onCancel }: Props) {
```

Replace with:

```tsx
const PROVIDER_TYPES: ProviderType[] = [
  "openai",
  "anthropic",
  "ollama",
  "openai-compatible",
  "github-copilot",
  "google-ai",
  "openrouter",
  "xai",
  "deepseek",
  "kimi",
  "anthropic-compatible",
];

function FormSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="form-section">
      <div className="form-section-title">{title}</div>
      {children}
    </div>
  );
}

export function ProviderForm({ existing, onSave, onCancel }: Props) {
```

- [ ] **Step 3: Verify it compiles so far**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors (the `FormSection` component is defined but not yet used — that's fine, it's exported implicitly as a module-scope function, not `export`ed, so TypeScript won't flag it as unused since Step 4 uses it in the same file).

- [ ] **Step 4: Replace the entire `return` statement**

Find the component's `return (` statement — it starts right after the last `useEffect`/handler (`runCopilotDeviceAuth`) and is the last thing in the component before the closing `}`. It currently starts with:

```tsx
  return (
    <div className="provider-form">
```

and ends with:

```tsx
      </div>
    </div>
  );
}
```

Replace the **entire** `return (...)` block (everything from `return (` through the matching `);` that closes the component function) with:

```tsx
  return (
    <div className="provider-form">
      <h3>{isEdit ? t.edit_provider : t.new_provider}</h3>

      {error && <div className="form-error">{error}</div>}

      <FormSection title={t.provider_section_basic}>
        <div className="form-group">
          <label>{t.provider_type}</label>
          <select
            value={providerType}
            onChange={(e) => setProviderType(e.target.value as ProviderType)}
            disabled={isEdit}
          >
            {PROVIDER_TYPES.map((pt) => (
              <option key={pt} value={pt}>
                {PROVIDER_TYPE_LABELS[pt]}
              </option>
            ))}
          </select>
        </div>

        <div className="form-group">
          <label>ID</label>
          <input
            type="text"
            value={id}
            onChange={(e) => setId(e.target.value)}
            placeholder="e.g. claude-sonnet"
            disabled={isEdit}
          />
        </div>

        <div className="form-group">
          <label>{t.provider_display_name}</label>
          <input
            type="text"
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            placeholder="e.g. Claude Sonnet"
          />
        </div>
      </FormSection>

      {providerType !== "ollama" && (
        <FormSection title={t.provider_section_auth}>
          {providerType === "anthropic" && (
            <div className="form-group">
              <label>{t.settings_provider_auth_type}</label>
              <div className="anthropic-auth-tabs">
                <button
                  type="button"
                  className={`auth-tab ${anthropicAuthMethod === "api_key" ? "active" : ""}`}
                  onClick={() => { setAnthropicAuthMethod("api_key"); setAnthropicOAuthCode(""); setAuthStatus(null); }}
                >
                  {t.settings_provider_auth_api_key}
                </button>
                <button
                  type="button"
                  className={`auth-tab ${anthropicAuthMethod === "oauth" ? "active" : ""}`}
                  onClick={() => { setAnthropicAuthMethod("oauth"); setApiKey(""); setAuthStatus(null); }}
                >
                  {t.settings_provider_auth_oauth("Claude Pro/Max")}
                </button>
              </div>
            </div>
          )}

          {providerType === "anthropic" && anthropicAuthMethod === "api_key" && (
            <div className="form-group">
              <label>{t.provider_api_key}</label>
              <input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={isEdit ? t.provider_api_key_placeholder_edit : t.provider_api_key_placeholder_new}
                autoComplete="off"
              />
            </div>
          )}

          {providerType === "anthropic" && anthropicAuthMethod === "oauth" && (
            <div className="form-group">
              <label>{t.settings_provider_auth_oauth("")}</label>
              {anthropicOAuthLoggedIn ? (
                <div className="anthropic-oauth-done">
                  <span className="anthropic-oauth-ok">{t.settings_provider_oauth_ok}</span>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    disabled={saving}
                    onClick={async () => {
                      if (!isEdit) return;
                      try {
                        await anthropicOAuthLogout(id.trim());
                        setAnthropicOAuthLoggedIn(false);
                        setAnthropicAuthMethod("api_key");
                      } catch (e: unknown) {
                        setAuthStatus(String(e));
                      }
                    }}
                  >
                    {t.settings_provider_oauth_logout}
                  </button>
                </div>
              ) : (
                <div className="anthropic-oauth-flow">
                  {authing ? (
                    <div className="anthropic-oauth-paste-ui">
                      <p className="anthropic-oauth-paste-hint">
                        {t.settings_provider_oauth_instructions}
                      </p>
                      <textarea
                        className="anthropic-oauth-url-input"
                        value={anthropicOAuthCode}
                        onChange={(e) => setAnthropicOAuthCode(e.target.value)}
                        placeholder={t.settings_provider_oauth_placeholder}
                        rows={2}
                        autoFocus
                      />
                      <div className="anthropic-oauth-btns">
                        <button
                          type="button"
                          className="aiterm-btn aiterm-btn--primary"
                          disabled={!anthropicOAuthCode.trim()}
                          onClick={async () => {
                            const pid = id.trim();
                            if (!pid) {
                              setAuthStatus(t.settings_provider_oauth_err_save_first);
                              setAuthing(false);
                              return;
                            }
                            try {
                              await anthropicOAuthComplete(pid, anthropicOAuthCode.trim());
                              setAnthropicOAuthLoggedIn(true);
                              setAuthStatus(t.settings_provider_oauth_success);
                              setAnthropicOAuthCode("");
                              fetchAnthropicModels(pid);
                            } catch (e: unknown) {
                              setAuthStatus(t.settings_provider_oauth_err(String(e)));
                            } finally {
                              setAuthing(false);
                            }
                          }}
                        >
                          {t.settings_provider_btn_confirm_auth}
                        </button>
                        <button
                          type="button"
                          className="aiterm-btn aiterm-btn--secondary"
                          onClick={() => { setAuthing(false); setAnthropicOAuthCode(""); }}
                        >
                          {t.settings_provider_btn_cancel}
                        </button>
                      </div>
                    </div>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="aiterm-btn aiterm-btn--primary"
                        disabled={!id.trim()}
                        onClick={async () => {
                          setAuthing(true);
                          setAuthStatus(null);
                          try {
                            await anthropicOAuthStart();
                          } catch (e: unknown) {
                            setAuthStatus(t.settings_provider_oauth_err(String(e)));
                            setAuthing(false);
                          }
                        }}
                      >
                        {t.settings_provider_btn_open_auth}
                      </button>
                      {!id.trim() && (
                        <div className="form-hint">{t.settings_provider_oauth_id_required}</div>
                      )}
                    </>
                  )}
                  {authStatus && (
                    <div className={`form-hint ${authStatus.startsWith("錯誤") || authStatus.startsWith("Error") ? "form-hint--error" : ""}`}>
                      {authStatus}
                    </div>
                  )}
                </div>
              )}
              {anthropicOAuthLoggedIn && authStatus && <div className="form-hint">{authStatus}</div>}
            </div>
          )}

          {providerType !== "ollama" && providerType !== "github-copilot" && providerType !== "anthropic" && (
            <div className="form-group">
              <label>
                {providerType === "openai-compatible" ? t.provider_api_key_optional : t.provider_api_key}
              </label>
              <input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={isEdit ? t.provider_api_key_placeholder_edit : t.provider_api_key_placeholder_new}
                autoComplete="off"
              />
            </div>
          )}

          {providerType === "github-copilot" && (
            <>
              <div className="form-group">
                <label>{t.provider_oauth_client_id}</label>
                <input
                  type="text"
                  value={oauthClientId}
                  onChange={(e) => setOauthClientId(e.target.value)}
                  placeholder={t.provider_oauth_client_id_placeholder}
                />
              </div>
              <div className="form-group">
                <label>{t.provider_auth_action}</label>
                <button
                  type="button"
                  className="aiterm-btn aiterm-btn--primary"
                  onClick={runCopilotDeviceAuth}
                  disabled={authing}
                >
                  {authing
                    ? t.provider_auth_running
                    : (apiKey.trim() || (isEdit && existing?.has_api_key))
                      ? t.provider_auth_ok
                      : t.provider_copilot_device_auth}
                </button>
                {authStatus && <div className="form-hint">{authStatus}</div>}
              </div>
            </>
          )}
        </FormSection>
      )}

      <FormSection title={t.provider_section_endpoint}>
        {(providerType === "ollama" ||
          providerType === "openai-compatible" ||
          providerType === "github-copilot" ||
          providerType === "google-ai" ||
          providerType === "anthropic-compatible") && (
          <div className="form-group">
            <label>{t.provider_base_url}</label>
            {providerType === "openai-compatible" && (
              <div className="presets">
                {COMPATIBLE_PRESETS.map((p) => (
                  <button
                    key={p.label}
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    onClick={() => setBaseUrl(p.url)}
                  >
                    {p.label}
                  </button>
                ))}
              </div>
            )}
            {providerType === "anthropic-compatible" && (
              <div className="presets">
                {ANTHROPIC_COMPATIBLE_PRESETS.map((p) => (
                  <button
                    key={p.label}
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    onClick={() => setBaseUrl(p.url)}
                  >
                    {p.label}
                  </button>
                ))}
              </div>
            )}
            <input
              type="text"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder={DEFAULT_BASE_URLS[providerType] || "https://..."}
            />
          </div>
        )}

        <div className="form-group">
          <label>{t.provider_model}</label>
          {providerType === "ollama" ? (
            ollamaLoading ? (
              <input type="text" value={t.provider_ollama_loading} disabled />
            ) : ollamaModels.length > 0 ? (
              <select value={model} onChange={(e) => setModel(e.target.value)}>
                {ollamaModels.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={t.provider_ollama_fallback_placeholder}
              />
            )
          ) : providerType === "github-copilot" ? (
            copilotLoading ? (
              <input type="text" value={t.provider_model_loading} disabled />
            ) : copilotModels.length > 0 ? (
              <select value={model} onChange={(e) => setModel(e.target.value)}>
                {copilotModels.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            ) : (
              <input
                type="text"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={DEFAULT_MODELS[providerType]}
              />
            )
          ) : providerType === "google-ai" ? (
            googleAiLoading ? (
              <input type="text" value={t.provider_model_loading} disabled />
            ) : (
              <>
                <input
                  type="text"
                  list="google-ai-models-list"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={googleAiModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
                />
                {googleAiModels.length > 0 && (
                  <datalist id="google-ai-models-list">
                    {googleAiModels.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                )}
              </>
            )
          ) : providerType === "openrouter" ? (
            openRouterLoading ? (
              <input type="text" value={t.provider_model_loading} disabled />
            ) : (
              <>
                <input
                  type="text"
                  list="openrouter-models-list"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={openRouterModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
                />
                {openRouterModels.length > 0 && (
                  <datalist id="openrouter-models-list">
                    {openRouterModels.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                )}
              </>
            )
          ) : providerType === "xai" ? (
            xaiLoading ? (
              <input type="text" value={t.provider_model_loading} disabled />
            ) : (
              <>
                <input
                  type="text"
                  list="xai-models-list"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={xaiModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
                />
                {xaiModels.length > 0 && (
                  <datalist id="xai-models-list">
                    {xaiModels.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                )}
              </>
            )
          ) : providerType === "deepseek" ? (
            deepseekLoading ? (
              <input type="text" value={t.provider_model_loading} disabled />
            ) : (
              <>
                <input
                  type="text"
                  list="deepseek-models-list"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={deepseekModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
                />
                {deepseekModels.length > 0 && (
                  <datalist id="deepseek-models-list">
                    {deepseekModels.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                )}
              </>
            )
          ) : providerType === "kimi" ? (
            kimiLoading ? (
              <input type="text" value={t.provider_model_loading} disabled />
            ) : (
              <>
                <input
                  type="text"
                  list="kimi-models-list"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={kimiModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
                />
                {kimiModels.length > 0 && (
                  <datalist id="kimi-models-list">
                    {kimiModels.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                )}
              </>
            )
          ) : providerType === "anthropic" && anthropicAuthMethod === "oauth" && anthropicOAuthLoggedIn ? (
            anthropicModelsLoading ? (
              <input type="text" value={t.settings_provider_model_loading_placeholder} disabled />
            ) : (
              <select value={model} onChange={(e) => setModel(e.target.value)}>
                {!anthropicModels.includes(model) && model && (
                  <option value={model}>{model}</option>
                )}
                {anthropicModels.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </select>
            )
          ) : (
            <input
              type="text"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder={DEFAULT_MODELS[providerType]}
            />
          )}
        </div>

        {(providerType === "openai-compatible" ||
          providerType === "github-copilot" ||
          providerType === "google-ai" ||
          providerType === "openrouter" ||
          providerType === "xai" ||
          providerType === "deepseek" ||
          providerType === "kimi") && (
          <div className="form-group form-group--checkbox">
            <label>
              <input
                type="checkbox"
                checked={supportsJsonMode}
                onChange={(e) => setSupportsJsonMode(e.target.checked)}
              />
              {t.provider_json_mode}
            </label>
          </div>
        )}
      </FormSection>

      <div className="form-actions">
        <button
          type="button"
          className="aiterm-btn aiterm-btn--secondary"
          onClick={onCancel}
          disabled={saving}
        >
          {t.cancel}
        </button>
        <button
          type="button"
          className="aiterm-btn aiterm-btn--primary"
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? t.saving_btn : t.save}
        </button>
      </div>
    </div>
  );
}
```

Note what changed vs. the original, precisely:
- Type/ID/Display Name wrapped in `<FormSection title={t.provider_section_basic}>`.
- Anthropic tabs, Anthropic API key, Anthropic OAuth flow, generic API key, and the GitHub Copilot block are now all inside one `<FormSection title={t.provider_section_auth}>`, itself gated on `providerType !== "ollama"`.
- The GitHub Copilot block is split from one `<div className="form-group">` containing two labeled things into two separate `<div className="form-group">` elements inside a `<>` fragment — this is the actual bug fix. No handler, condition, or text changed.
- The Base URL block, Model block, and JSON-mode checkbox are now inside one `<FormSection title={t.provider_section_endpoint}>`. The giant Model ternary is byte-for-byte unchanged internally.
- `className` changed on: the Anthropic OAuth logout button (`anthropic-oauth-logout` → `aiterm-btn aiterm-btn--secondary aiterm-btn--sm`), the Anthropic OAuth "confirm"/"open browser" buttons (`anthropic-oauth-open` → `aiterm-btn aiterm-btn--primary`, both occurrences), the Anthropic OAuth cancel button (`anthropic-oauth-cancel-btn` → `aiterm-btn aiterm-btn--secondary`), the GitHub Copilot device-auth button (no class → `aiterm-btn aiterm-btn--primary`), both preset-button loops (`preset-btn` → `aiterm-btn aiterm-btn--secondary aiterm-btn--sm`), and the bottom Cancel/Save buttons (no class / `btn-primary` → `aiterm-btn aiterm-btn--secondary` / `aiterm-btn aiterm-btn--primary`).
- Nothing else — no prop, handler, condition, or text changed anywhere.

- [ ] **Step 5: Verify it compiles**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors.

- [ ] **Step 6: Confirm no dangling references to deleted CSS classes**

Run: `grep -n "preset-btn\|btn-primary\|anthropic-oauth-open\|anthropic-oauth-logout\|anthropic-oauth-cancel-btn" src/components/Settings/ProviderForm.tsx`
Expected: no matches.

- [ ] **Step 7: Lint**

Run: `npx eslint src/components/Settings/ProviderForm.tsx`
Expected: same error/warning count and content as before this task's changes (this file already has pre-existing `react-hooks/set-state-in-effect` findings from earlier work, unrelated to this task — confirm you haven't added new ones by comparing against `git stash` + re-lint if unsure).

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/ProviderForm.tsx
git commit -m "feat(settings): group provider form fields into sections and fix cramped Copilot auth layout"
```

---

## Task 4: Widen the provider form modal

**Files:**
- Modify: `src/components/Settings/ProvidersPage.css:135`

- [ ] **Step 1: Change the panel width**

Find:

```css
.provider-form-panel {
  background: #141414;
  border: 1px solid #2e2e2e;
  border-radius: 8px;
  max-height: 90vh;
  overflow-y: auto;
  padding: 24px;
  width: 480px;
}
```

Replace with:

```css
.provider-form-panel {
  background: #141414;
  border: 1px solid #2e2e2e;
  border-radius: 8px;
  max-height: 90vh;
  overflow-y: auto;
  padding: 24px;
  width: 560px;
}
```

- [ ] **Step 2: Verify**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors (this is a CSS-only change; this check just confirms nothing else broke).

- [ ] **Step 3: Commit**

```bash
git add src/components/Settings/ProvidersPage.css
git commit -m "style(settings): widen provider form modal to 560px for the new sectioned layout"
```

---

## Task 5: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Full type check**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors.

- [ ] **Step 2: Full lint**

Run: `npm run lint`
Expected: no new errors/warnings introduced by this plan's changes (compare the total count against a `git stash` baseline if unsure — this codebase has pre-existing lint findings unrelated to this feature; the bar is "didn't add any new ones," not "zero total").

- [ ] **Step 3: Full frontend test suite**

Run: `npm run test -- --run`
Expected: same pass count as before this plan (no test file exists for `ProviderForm.tsx`/`ProvidersPage.tsx`, so no test count change is expected either way — this just guards against an unrelated regression).

- [ ] **Step 4: Manual verification checklist**

Run `npm run tauri:dev` (or otherwise launch the app) and open Settings → AI 供應商 → 新增供應商. For as many of the 11 provider types as practical, confirm:
1. Only the sections that have applicable fields for that type are shown (e.g. Ollama shows no "驗證方式" section at all).
2. The GitHub Copilot "驗證方式" section shows the OAuth Client ID field and the device-auth button as two visually separated rows, not cramped together.
3. Buttons (儲存, 取消, device-auth, OAuth open/confirm/cancel/logout, preset buttons) all render with the same purple-gradient/dark-secondary look used elsewhere in the app (e.g. compare against the provider list page's 測試/編輯/移除 buttons).
4. Field labels are no longer in shouty all-caps.
5. The modal is visibly wider (560px) and nothing overflows or misaligns at that width, including with `max-height: 90vh` scrolling on a small window.

Report explicitly which of these were checked against a live running app versus verified only by reading the code, if a live GUI session isn't available in your environment — this is a purely visual change and code review alone cannot fully substitute for looking at it.

- [ ] **Step 5: Report unresolved concerns, if any**

If Step 4 surfaces any visual issue (e.g. a section that doesn't look right, spacing that's off), do not silently patch it outside this plan's scope — report it back for a quick follow-up fix rather than improvising a redesign.
