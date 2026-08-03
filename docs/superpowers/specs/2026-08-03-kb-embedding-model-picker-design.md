# 知識庫 Embedding Model 選擇器 — 設計

日期：2026-08-03
狀態：已核可，待寫實作計畫

## 問題

新增筆記本時，「Embedding Model」是一個空白輸入框，使用者必須自己知道並手打模型名稱。實務上有兩個後果：

**打錯或選錯不會當場發現。** 填進去的字串直到第一次 ingest 才會被使用。若那個模型不存在，或它是聊天模型而非 embedding 模型，錯誤要等到文件跑到一半才浮現。

**「Embedding Provider」欄位容易被誤讀成模型。** provider 的顯示名稱是使用者在設定裡自取的，很多人會用模型名來命名（實際案例：一個名為 `Qwen3.6-35B-A3B-4bit` 的 OpenAI-Compatible provider）。畫面上先出現一個看起來像模型名的欄位，下面才是真正的模型欄位，順序本身就在誤導。

順帶一提，`notebooks.embed_dim` 這個欄位從建立至今**從未被寫入**。唯一會寫它的 `set_notebook_embed_config` 是死程式碼，全庫沒有呼叫者。

## 範圍

**含：** 新增筆記本時的模型列舉、排序、建立前探測驗證、`embed_dim` 寫入、provider 選項標示。

**不含：** 更換現有筆記本的 embedding 模型。那需要清空並重建索引，還要處理進度、中斷與失敗回復，是另一個完整功能。本次的探測與 `embed_dim` 寫入正好是它的地基。

## 設計

### 後端

**`knowledge_base/embedding.rs`** — 新增模型列舉，刻意與既有的 `HttpEmbedder::embed` 共用同一個 match 形狀：

```rust
pub async fn list_models(cfg: &EmbedderConfig) -> Result<Vec<String>, String> {
    match cfg.provider_type {
        ProviderType::Ollama => // 複用 OllamaClient::list_models()
        ProviderType::Openai | ProviderType::OpenaiCompatible => list_openai_compatible(cfg).await,
        other => Err(format!("{other} 不支援 embedding")),
    }
}
```

`list_openai_compatible` 打 `GET {base_url}/models`，解析 `{"data":[{"id":...}]}` 取 id。OpenAI 與 OpenAI-Compatible 共用這一份。

兩個函式必須對 provider 類型做出一致的判斷，放在同一檔案的相鄰位置是為了讓它們不會各自漂移。

**`commands/knowledge_base.rs`**

- 新增 `kb_list_embedding_models(provider_id) -> Vec<String>`，沿用既有的 `resolve_embedder_config`
- 修改 `kb_create_notebook`：寫入 DB 前先呼叫 `HttpEmbedder::embed(&["test"])` 探測一次。成功則取 `vec[0].len()` 作為維度；失敗直接回 `Err`，不碰 DB

探測放在 create 內部而非獨立指令，是為了一個不變式：**不可能存在「模型未經驗證」的筆記本**。若探測是獨立指令，前端有可能跳過它。

**`db/knowledge_base.rs`** — `create_notebook` 增加 `embed_dim: i64` 參數，INSERT 一併寫入。

**`lib.rs`** — 註冊 `kb_list_embedding_models`（use 與 invoke_handler 兩處）。

死程式碼 `set_notebook_embed_config` 保留不動。它是未來「更換模型」功能的地基，不在本次範圍內。

### 前端

**`src/ipc/knowledgeBase.ts`** — 新增 `listEmbeddingModels(providerId): Promise<string[]>`。`createNotebook` 的簽名不變，維度由後端算出，前端不傳。

**`NotebookCreateDialog.tsx`**

Provider 的 `<option>` 附上類型與 endpoint，例如 `Qwen3.6-35B-A3B-4bit — OpenAI-Compatible · localhost:1234`。資訊放在混淆發生的位置，比在欄位下方加說明文字更直接，也順帶解決「同類型多個 provider 分不出來」。

`ProviderInfo` 已經帶有 `provider_type` 與 `base_url`，不需要新增 IPC。`base_url` 為 null 時（OpenAI 與 Ollama 使用預設值的情況）顯示該類型的預設位址，與 `resolve_embedder_config` 的 fallback 一致，避免畫面上出現空白的 endpoint。

Model 欄位從純 `<input>` 改為 `<input list>` + `<datalist>`。這是 `ProviderForm.tsx` 已經在 5 處使用的既有模式，原生元件不需自理鍵盤操作與無障礙。

`<datalist>` 無法顯示群組標題，因此分組降級為排序：名稱含 `embed` / `bge` / `gte` / `e5` / `nomic` / `minilm` / `mxbai` / `jina` 的排在前面，其餘在後。這個啟發式**只影響排序，不影響可選性**——冷門的 embedding 模型仍在清單裡，只是排得比較後面，而使用者永遠可以直接手打。

清單在 `providerId` 改變時重新載入，載入中顯示 disabled 輸入框（沿用 ProviderForm 的 `t.provider_model_loading`）。

### 錯誤處理

| 情境 | 行為 |
|---|---|
| 列舉端點 404 或不存在 | 靜默降級為純文字輸入 |
| 列舉網路錯誤或逾時 | 靜默降級為純文字輸入 |
| provider 類型不支援 embedding | 不會發生，dialog 已用 `EMBEDDING_CAPABLE_TYPES` 過濾 |
| 探測失敗 | 顯示錯誤，不建立筆記本，dialog 保持開啟 |
| 探測回傳空向量或維度 0 | 視為失敗，同上 |
| 探測逾時 | `HttpEmbedder` 既有的 60 秒 timeout |

分界：**列舉失敗不擋人，探測失敗一定擋。** 前者是便利功能，不少自架服務根本沒有 `/v1/models`，跳錯誤只是噪音；後者關係到資料正確性，錯了就是整批向量報廢。

**但「回傳錯誤」不等於「什麼都沒建立」。** `create_notebook` 是 INSERT 之後再 SELECT 讀回整列，兩者不是同一個交易。INSERT 成功而後續 SELECT 失敗（SQLITE_BUSY、連線中斷）時會回傳 `Err`，可是那一列已經在資料庫裡了——而 `notebooks` 沒有 `folder_path` 的 UNIQUE 約束，使用者重試就會建出第二筆。

不變式仍然成立（存在的每一列都確實通過過探測），但方向是單向的。因此**前端在建立失敗時必須無條件重新載入筆記本列表**，不能因為收到 `Err` 就假設列表沒變。這個 INSERT-then-SELECT 的非原子性本身不在本次範圍（乾淨的修法是 `INSERT ... RETURNING`），這裡只處理它的後果。

### 測試

Rust（`src-tauri/tests/`，wiremock）：

- `list_openai_compatible` 正確解析 `{"data":[{"id":...}]}`
- 列舉端點回 404 時回 `Err`，降級由呼叫端決定
- 不支援的 provider 類型回 `Err`
- **探測失敗時不寫入 DB** — 核心不變式
- 探測成功時 `embed_dim` 寫入正確值

前端（Vitest + React Testing Library）：

- 切換 provider 觸發重新載入清單
- 列舉失敗時仍可手打並成功送出
- 疑似 embedding 的模型排在前面
- provider 選項顯示類型與 endpoint
- `base_url` 為 null 的 provider 顯示預設位址而非空白

依 TDD，每個測試都要先確認會紅再寫實作。

## 這個設計不保證什麼

實作 Task 3 時發現的限制，寫在這裡以免這個功能被描述成比實際更強。

**探測證明的是「這個模型會在 embeddings 端點回傳一個非空向量」，不是「這是一個好的 embedding 模型」。**

它可靠攔下的是：

- 模型名稱打錯（模型不存在 → 端點回錯誤）
- 選到的 provider 根本沒有 embeddings 端點
- provider 設定本身壞掉（金鑰失效、base_url 錯誤）

它攔不住的是：**使用者選了聊天模型，而 gateway 照單全收。** LM Studio 一類的自架服務往往會拿聊天模型的 encoder 生出一個形狀正確的向量並回傳 200。探測看到非空向量就會放行，筆記本照樣建立，`embed_dim` 也會記下那個維度——然後檢索品質莫名其妙地差，而且沒有任何錯誤訊息。

這正好是這個功能最初想解決的情境之一，所以要講清楚：**在這條路徑上，清單的排序啟發式不是輔助提示，而是唯一的防線。** 本文件前面把排序描述為「只是輔助」、把探測描述為「真正擋下誤選的關卡」——那個說法對「打錯字」成立，對「gateway 照單全收」不成立。

沒有便宜的辦法可以在建立當下分辨兩者。可考慮的方向（都不在本次範圍）：對同一模型送兩段語意相近與相反的文字，比較 cosine 距離是否符合預期——但那要多次往返，且門檻難以在不同模型間通用，偽陽性的代價（擋下正常模型）比目前的漏網更糟。

## 已考慮並否決的方案

**只做名稱過濾，不探測。** 用關鍵字濾掉聊天模型，實作最快。否決原因：名稱不含關鍵字的 embedding 模型會被誤濾而完全消失，比現況更糟；而且完全擋不住手打錯誤。探測能攔下打錯字與端點不支援，那是過濾做不到的——但見上一節，它同樣有攔不住的情況，兩者是互補而非取代關係。

**全部列出，不過濾也不探測。** 改動最小，只把 input 換成 combobox。否決原因：誤選要到 ingest 跑一半才發現，沒有解決核心問題。

**擴充 `commands/provider.rs` 的既有指令家族。** 新增 `get_openai_compatible_models(id)` 與既有 9 支並列，前端自行依 provider 類型決定呼叫哪支。否決原因：會把「哪些類型支援 embedding」的知識複製到前端第二處，與 `EMBEDDING_CAPABLE_TYPES` 形成兩個必須同步的來源。代價是日後 Settings 若也需要列舉 OpenAI-Compatible 的模型，得把 `/v1/models` 抽出去共用——以目前需求判斷值得。

**自訂下拉元件保留完整分組。** 像 `ModelPickerButton` 用 portal 自繪，可以做出群組標題。否決原因：與 `ProviderForm` 已在 5 處使用的 datalist 模式分岔，且要自理鍵盤操作與無障礙。分組只是輔助提示，擋下誤選的是探測那道關卡。
