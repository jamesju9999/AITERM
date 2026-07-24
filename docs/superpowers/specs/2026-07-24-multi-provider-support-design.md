# 新增 AI Provider 支援：OpenRouter / xAI / DeepSeek / Kimi / Anthropic-Compatible

**日期**：2026-07-24
**狀態**：待審閱

## 背景與目標

AITerm 目前的 `ai/` provider 架構支援 OpenAI、Anthropic（含 OAuth）、Ollama、通用 OpenAI-Compatible、GitHub Copilot、Google AI 六種類型。本次目標是新增五種 provider：

- **OpenRouter**、**xAI (Grok)**、**DeepSeek**、**Kimi (Moonshot)** —— 皆採 API Key 認證（不做 OAuth／訂閱帳號重用），各自成為獨立的 `ProviderType`。
- **Anthropic Compatible** —— 通用型，讓使用者接任何相容 Anthropic Messages API 格式的第三方端點（含 Kimi Coding 這種走 Anthropic 格式的訂閱制服務）。

決策依據：先前已透過研究確認這四家 API 皆為 OpenAI Chat Completions 相容格式，且 AITerm 現有的 `OpenaiCompatible` 通用型與 `GoogleAi`（其底層也是 `OpenAiCompatibleClient` + 固定 base_url）已經證明「固定 base_url + 重用既有 client」這個模式可行且低風險。`AnthropicClient::with_base_url` 也早已支援任意 base_url，Anthropic Compatible 型別可以零新增 client 程式碼。

## 範圍界定（重要決策，已與使用者確認）

| 決策點 | 選擇 |
|---|---|
| OpenRouter/xAI/DeepSeek/Kimi 認證方式 | **只做 API Key，不做 OAuth**（不移植 OmniRoute 裡偽裝官方 CLI 指紋的 reverse-engineered OAuth） |
| 這四家的呈現方式 | **各自獨立的 `ProviderType`**（非「OpenAI-Compatible 底下的預設按鈕」）——底層仍重用 `OpenAiCompatibleClient` |
| 既有 `COMPATIBLE_PRESETS` 裡的 OpenRouter/DeepSeek | **移除**，避免同一件事有兩種做法 |
| 模型清單 | **動態抓取**（比照 Ollama/GitHub Copilot/Google AI 的模式），非純自由文字 |
| Anthropic Compatible 呈現方式 | **獨立的 `ProviderType`「anthropic-compatible」**，比照 `openai` / `openai-compatible` 的關係 |
| Kimi Coding | 列入本次範圍，作為 Anthropic Compatible 底下的預設快選按鈕（`https://api.kimi.com/coding`） |

### 明確排除（Non-goals）

- 不做任何 OAuth／裝置碼／瀏覽器登入流程給這五個新 provider。
- 不移植 OmniRoute 裡的 User-Agent／header 指紋偽裝技術。
- Anthropic Compatible 型別不做動態模型清單抓取（維持自由文字輸入，因為這是「使用者自帶端點」的通用型，沒有單一官方模型清單可查）。
- 不保證 Kimi Coding 預設按鈕開箱即用——已知 OmniRoute 的實作會在 URL 後面加 `?beta=true` 查詢字串，而 AITerm 目前的 URL 拼接邏輯（`base_url + "/v1/messages"`）不支援附加查詢字串。此為已知風險，實作時需要用真實金鑰測試確認是否為必要參數；若非必要則無影響，若必要則需另外設計 URL 拼接的彈性（不在本次範圍內解決，先以「盡力而為的預設值」上線）。
- 不保證 OpenRouter/xAI/DeepSeek/Kimi 四家都真的有公開 `/models` 端點——高機率有（皆為 OpenAI 慣例相容 API），但只有 OpenRouter 確定查過。若某家沒有，該家的動態抓取會失敗並優雅退回自由文字輸入（不影響其他家）。

## 後端設計（Rust，`src-tauri/src/`）

### 1. `config/types.rs`

`ProviderType` enum 新增 5 個 variant：

```rust
pub enum ProviderType {
    Openai,
    Anthropic,
    Ollama,
    OpenaiCompatible,
    GithubCopilot,
    GoogleAi,
    Openrouter,
    Xai,
    Deepseek,
    Kimi,
    AnthropicCompatible,
}
```

沿用既有 `#[serde(rename_all = "kebab-case")]`，自動序列化為 `openrouter`/`xai`/`deepseek`/`kimi`/`anthropic-compatible`，無需手動 rename。`Display` impl 補上對應的顯示字串。`ProviderConfig` 結構不需新增欄位——`base_url`／`model`／`auth_method` 都已通用。

### 2. `ai/router.rs`

新增 5 個 match arm：

- `Openrouter | Xai | Deepseek | Kimi`：建構 `OpenAiCompatibleClient`，base_url 未填時分別預設為 `https://openrouter.ai/api/v1`、`https://api.x.ai/v1`、`https://api.deepseek.com/v1`、`https://api.moonshot.ai/v1`。API Key **必填**（比照 `Openai`/`GoogleAi` 的寫法：`self.secrets.get(...)?.ok_or(AiError::NotConfigured)?`），不像通用 `OpenaiCompatible` 把 key 當可選。
- `AnthropicCompatible`：直接重用 `AnthropicClient::with_base_url(key, model, base_url)`。base_url 未填時回傳 `AiError::Network`（比照 `OpenaiCompatible` 現有的「base_url 必填」錯誤處理）。API Key 必填。不檢查／不支援 `auth_method == "oauth"`——這個型別永遠是 API Key 模式。

### 3. `commands/provider.rs`：動態模型清單指令

抽出共用 helper（從現有 `list_github_copilot_models` 泛化而來）：

```rust
async fn list_openai_style_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String>
```

打 `{base_url}/models`，bearer auth，解析既有的 `OpenAiModelsResponse { data: Vec<{ id: String }> }`，回傳 `Vec<String>`。

四家各自兩個薄 wrapper（直接傳 key 版 + 依 provider id 查已存 key 版，比照 Google AI 的兩函式模式）：

```
get_openrouter_models / get_openrouter_models_by_provider
get_xai_models / get_xai_models_by_provider
get_deepseek_models / get_deepseek_models_by_provider
get_kimi_models / get_kimi_models_by_provider
```

共 8 個新 `#[tauri::command]` + 1 個共用 helper。`_by_provider` 版本需檢查 `provider.provider_type` 是否為對應型別，錯誤訊息比照現有 Google AI 版本的寫法。

### 4. `lib.rs`

把上述 8 個指令加進 import 區塊與 `invoke_handler!(tauri::generate_handler![...])` 清單。

## 前端設計（`src/`）

### 1. `src/ipc/config.ts`

`ProviderType` union 新增 `"openrouter" | "xai" | "deepseek" | "kimi" | "anthropic-compatible"`。

### 2. `src/ipc/provider.ts`

- `PROVIDER_TYPE_LABELS` 新增：`OpenRouter`、`xAI (Grok)`、`DeepSeek`、`Kimi (Moonshot)`、`Anthropic-Compatible`。
- `DEFAULT_BASE_URLS` 新增四家固定端點；`anthropic-compatible: ""`。
- `DEFAULT_MODELS`：xAI/DeepSeek/Kimi 給常見旗艦模型當 placeholder；OpenRouter 與 `anthropic-compatible` 給空字串。
- **`COMPATIBLE_PRESETS`**：移除 `OpenRouter`、`DeepSeek` 兩筆，只留 `LM Studio`、`vLLM`。
- 新增 `ANTHROPIC_COMPATIBLE_PRESETS = [{ label: "Kimi Coding", url: "https://api.kimi.com/coding" }]`。
- 新增 8 個 IPC 包裝函式，對應後端的 8 個新指令。

### 3. `src/components/Settings/ProviderForm.tsx`

- `PROVIDER_TYPES` 陣列加入 5 個新值。
- **base_url 欄位顯示規則**：`openrouter`/`xai`/`deepseek`/`kimi` **不顯示**（比照 `openai` 型別，端點固定不給使用者改；進階自訂端點需求走既有的 `openai-compatible` 型別）。`anthropic-compatible` **顯示** base_url 欄位，並在其上加 `ANTHROPIC_COMPATIBLE_PRESETS` 的快選按鈕（沿用既有 preset-按鈕 UI，只是資料來源換成新清單）。
- **模型欄位**：四個新 API-Key provider 一律採用 `<input list=... />` + `<datalist>`（比照現有 Google AI 的做法），而非強制 `<select>`——自由輸入 + 動態抓回清單當建議。抓取時機：填入 API Key 後 debounce 500ms 自動呼叫；編輯模式下若已存過 key 則直接呼叫（同 Google AI 現有 `useEffect` 邏輯，四家各自一組 `useEffect` + state）。
- `anthropic-compatible` 的模型欄位維持純自由文字輸入，不接動態抓取。
- API Key 欄位、JSON mode checkbox 沿用既有通用邏輯，不需特別改動。

## 錯誤處理

- 動態模型抓取失敗（金鑰錯誤、網路問題、端點不存在 `/models`）：`catch` 後清空建議清單、退回自由文字輸入，不阻擋使用者存檔——與 Google AI 現有行為一致。
- 執行期缺 API Key：沿用既有 `AiError::NotConfigured`，不新增 error kind。
- `AnthropicCompatible` 缺 base_url：沿用 `OpenaiCompatible` 的錯誤訊息格式（`AiError::Network { message: "provider '{id}' has no base_url configured" }`）。

## 測試計畫

- Rust：擴充 `config/types.rs` 現有的 `provider_type_roundtrips_toml` 參數化測試，加入 5 個新 variant 的序列化字串斷言。
- Rust：`router.rs` 為每個新 match arm 加一個「缺 API Key 回 `NotConfigured`」的單元測試（比照現有 `unknown_provider_id_returns_not_configured` 的風格），**不**寫呼叫真實外部 API 的整合測試。
- 前端：暫無自動化測試涵蓋 `ProviderForm.tsx`，本次不新增測試框架，僅手動驗證（新增 provider → 填 Key → 模型建議清單出現 → 儲存 → 出現在清單並可設為預設）。

## 待驗證假設（實作階段需確認，不阻擋設計核准）

1. OpenRouter（已知有）、xAI、DeepSeek、Kimi 是否都有公開 `GET /models` 端點——多語言慣例上高機率有，實作時逐一用真實金鑰測一次。
2. Kimi Coding 端點是否真的需要 `?beta=true` 查詢字串才能正常運作——若需要，屬於快速後續修正項目，不影響本次其餘範圍。
