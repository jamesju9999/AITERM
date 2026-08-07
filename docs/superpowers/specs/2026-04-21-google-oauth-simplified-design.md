# Google Gemini OAuth — 簡化登入設計 spec

**日期：** 2026-04-21
**狀態：** 待實作

---

## 目標

移除使用者需要手動填入 Google Cloud Console 憑證的步驟。使用者只需點「使用 Google 登入」，完成瀏覽器授權後即可使用 Gemini。Token 自動刷新，使用者幾乎不需要重新登入。

---

## 方案選擇

使用 Gemini CLI（google-gemini/gemini-cli）公開原始碼中的 OAuth client 憑證，內建至 AITerm。Gemini CLI 的 client 為「桌面應用程式」類型，已通過 Google 驗證，可直接顯示「繼續使用 Gemini Code Assist and Gemini CLI」帳戶選擇畫面。

---

## 架構

### 1. 內建憑證（Rust 常數）

在 `src-tauri/src/commands/provider.rs` 新增兩個私有常數：

```rust
const GEMINI_CLI_CLIENT_ID: &str = "681255809395-oo8ft2oprdrnlhp2gq79no0vc7m4i8t2.apps.googleusercontent.com";
const GEMINI_CLI_CLIENT_SECRET: &str = "<從 Gemini CLI 原始碼取得>";
```

這些值取自 Gemini CLI 的公開 GitHub repo，與使用者在截圖中看到的相同。

### 2. OAuth 流程變更

`google_gemini_oauth_auth` 命令簽名移除 `client_id` / `client_secret` 參數：

```rust
pub async fn google_gemini_oauth_auth(
    provider_id: String,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String>
```

OAuth URL 改為 `access_type=offline`（取得 refresh token）：

```
https://accounts.google.com/o/oauth2/v2/auth
  ?client_id=GEMINI_CLI_CLIENT_ID
  &redirect_uri=http://localhost:{port}
  &response_type=code
  &scope=https://www.googleapis.com/auth/generative-language
  &access_type=offline
  &prompt=consent
```

Token exchange 回應解析 `access_token`、`refresh_token`、`expires_in`，計算 `expires_at = now + expires_in`，序列化為 JSON 存入 keychain：

```json
{
  "access_token": "ya29.xxx",
  "refresh_token": "1//xxx",
  "expires_at": 1745123456
}
```

Keychain key 維持 `{provider_id}`（與現有一致，但值從純字串改為 JSON 物件）。

**遷移行為：** 舊版已儲存的純字串 access token 無法解析為 JSON，router 回傳 `NotConfigured`，使用者需重新點一次「使用 Google 登入」完成遷移。屬預期行為（只需登入一次）。

### 3. Router 自動刷新（`src-tauri/src/ai/router.rs`）

`GoogleGeminiOauth` case 新增 helper `get_fresh_google_token`：

```
1. secrets.get(provider_id) → JSON blob
2. 若解析失敗或無 refresh_token → return Err(NotConfigured)
3. 若 access_token 未過期（expires_at > now + 60s）→ return access_token
4. 否則：POST https://oauth2.googleapis.com/token
     client_id, client_secret, refresh_token, grant_type=refresh_token
5. 更新 keychain（新 access_token + expires_at，refresh_token 不變）
6. return 新 access_token
```

刷新失敗（refresh token 已撤銷）→ 回傳 `AiError::AuthFailed`，前端收到後顯示「請重新登入 Google」。

### 4. Frontend 變更（`ProviderForm.tsx`）

`google-gemini-oauth` 區塊：

**移除：**
- Client ID 輸入欄 + label
- Client Secret 輸入欄 + label
- 說明文字（提示去 Google Cloud Console 的 hint）

**保留：**
- 「OAuth 身份驗證」section 標題
- 「使用您的 Google 帳戶登入，透過 OAuth 使用 Gemini」說明
- 「使用 Google 登入」按鈕
- 登入狀態文字（authing / authStatus）

`runGoogleOauth` 函式不再傳 `clientId`、`clientSecret`。

### 5. IPC 更新（`src/ipc/provider.ts`）

```ts
// 移除 clientId / clientSecret 參數
export function googleGeminiOauthAuth(providerId: string): Promise<string>
```

### 6. i18n 清理（`src/lib/i18n.ts`）

移除：
- `provider_google_client_id`
- `provider_google_client_id_placeholder`
- `provider_google_client_secret`
- `provider_google_client_secret_placeholder`
- `provider_google_oauth_hint`
- `err_google_client_id_required`
- `err_google_client_secret_required`

---

## 資料流

```
使用者點「使用 Google 登入」
  → frontend: googleGeminiOauthAuth(providerId)
  → Rust: 開啟 loopback server → open_browser(auth_url with offline scope)
  → 使用者在瀏覽器選擇帳戶 → 授權
  → Google redirect 到 localhost:{port}?code=xxx
  → Rust: exchange code → {access_token, refresh_token, expires_at}
  → keychain.set(provider_id, JSON)
  → return access_token 給前端（顯示 ✓ 登入成功）

使用者發 AI 請求
  → router.resolve_by_id(id)
  → get_fresh_google_token():
      若未過期 → 直接用 access_token
      若過期   → refresh → 更新 keychain → 用新 access_token
  → OpenAiCompatibleClient(access_token)
```

---

## 錯誤處理

| 情境 | 行為 |
|------|------|
| 使用者取消瀏覽器授權 | timeout 或 no code → 前端顯示錯誤訊息 |
| refresh token 已撤銷 | `AiError::AuthFailed` → 前端顯示「請重新登入 Google」 |
| 網路錯誤（refresh 失敗） | `AiError::Network` → 正常錯誤處理 |
| keychain 讀取失敗 | `AiError::NotConfigured` → 前端顯示未設定狀態 |

---

## 不在範圍內

- 向使用者顯示已登入的 Google 帳號 email
- Revoke token 的登出按鈕
- 多 Google 帳號切換
- `google-ai`（API key 模式）不受影響

---

## 檔案變動清單

| 檔案 | 變動 |
|------|------|
| `src-tauri/src/commands/provider.rs` | 新增 2 個常數；修改命令簽名與實作 |
| `src-tauri/src/ai/router.rs` | 新增 `get_fresh_google_token` helper；修改 `GoogleGeminiOauth` case |
| `src/ipc/provider.ts` | 移除 IPC 函式的 clientId/clientSecret 參數 |
| `src/components/Settings/ProviderForm.tsx` | 移除輸入欄與相關邏輯 |
| `src/lib/i18n.ts` | 移除過時的 i18n key |
