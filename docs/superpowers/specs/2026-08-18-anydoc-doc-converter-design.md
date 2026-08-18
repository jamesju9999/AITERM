# 文件轉換引擎 — 整合 anydoc — 設計

日期：2026-08-18
狀態：待使用者核可

## 問題

AITerm 目前唯一的文件轉換引擎是 MarkItDown（Python，透過 `python_env` 的 `DocCore`/`DocAudio` profile 跑在 uv 管理的 venv 裡），用於兩個地方：知識庫同步（`knowledge_base/ingest.rs` 的 `DocumentConverter` trait）與手動轉檔工具（`commands/markitdown.rs` 的 `markitdown_convert` command）。

[anydoc](https://github.com/firecrawl/anydoc) 是一個純 Rust 的文件轉換 crate，不需要 Python，對 docx/pdf/pptx/xlsx/csv/epub 等格式的轉換品質（官方以 LLM 裁判對 100 份真實文件評分）與速度（中位數 4.4ms，MarkItDown 同批格式 134.8ms）都明顯優於 MarkItDown，且額外支援 `.doc`/`.ppt`/`.odt`/`.ods`/`.odp`/`.rtf`——這些格式目前 AITerm 完全不支援。

但 anydoc 做不到 MarkItDown 現在做的事：圖片（走設定的 AI provider 做 vision OCR/描述）、音檔轉錄、`.msg`（Outlook 信件）、html、純文字類格式。這些必須留在 MarkItDown。

## 範圍

**含：**

- 新增 `anydoc` 作為第二個轉換引擎，依副檔名路由：anydoc 涵蓋的格式優先走 anydoc，其餘維持走 MarkItDown
- 擴充支援格式清單，收入 anydoc 額外支援的 `.doc`/`.ppt`/`.odt`/`.ods`/`.odp`/`.rtf`，以及 anydoc 官方格式表裡屬於同一格式家族的副檔名變體（如 `.docm`/`.pptm`/`.ppsx`/`.ppsm`/`.pot`/`.pps`/`.xlsm`/`.xlsb`）——這些變體不是使用者在 brainstorm 中逐一確認過的，但都是 anydoc 原生支援、且已經在同一張路由表裡的既有格式家族延伸，一併收入不增加額外實作成本
- 轉換失敗時的自動 fallback：anydoc 對「它該支援」的格式轉換失敗，自動改用 MarkItDown 重試
- 設定頁新增二選一選項：自動（anydoc 優先，預設）／只用 MarkItDown（舊行為，完全不碰 anydoc）
- 手動轉檔工具的 command/檔名改名（`markitdown_convert` → `document_convert`），反映新的路由行為
- 知識庫同步與手動轉檔工具共用同一套路由/fallback 邏輯

**不含（範圍外，先不做）：**

- 每個知識庫筆記本各自選引擎——這次是全域設定，跟 `submit_shortcut` 等既有設定同一層級
- 「只用 anydoc」（完全不回退 MarkItDown）的第三個選項——會讓圖片/音檔/msg/html 直接轉不出來，沒有實際用途
- 可插拔的多引擎抽象（chain of responsibility）——只有兩個引擎，這種彈性是投機性的，不做
- 修改 `python_env`/`DocCore`/`DocAudio` profile 本身或 `tools/MarkItDown/converter.py`——MarkItDown 那條路徑的實作不變，只是被呼叫的頻率降低
- 轉換成功時向使用者標示用了哪個引擎——只有兩個引擎都失敗時才需要在錯誤訊息裡交代

## 決策紀錄

以下是使用者在 brainstorm 中明確選定的：

| 決策 | 選定 |
|---|---|
| 新格式範圍 | 一併加入 anydoc 額外支援的 6 種格式（doc/ppt/odt/ods/odp/rtf） |
| 設定套用層級 | 全域設定（非各筆記本各自設定） |
| 轉換失敗處理 | 自動改用另一個引擎重試 |
| 設定選項數量 | 兩選一：自動(anydoc優先) / 只用 MarkItDown |
| 手動轉檔工具命名 | 改名為 `document_convert`，反映新路由行為 |

## 架構

### 共用路由模組

新增 `src-tauri/src/document_convert/` 模組，取代原本散在 `commands/markitdown.rs` 與 `commands/knowledge_base.rs` 裡的轉換邏輯：

```rust
enum Engine { Anydoc, MarkItDown }

/// 純函式，依副檔名決定走哪個引擎。不牽涉任何 I/O，方便單元測試涵蓋整張路由表。
fn engine_for_extension(ext: &str) -> Engine;

/// 實際執行轉換，內含 Auto 模式下的 fallback 邏輯。
async fn convert_document(
    app: AppHandle,
    path: &Path,
    vision_provider_id: Option<String>,
    engine_pref: DocConvertEngine,
) -> Result<String, String>;
```

`convert_document` 是唯一的執行入口，被兩個呼叫端共用，路由規則和 fallback 邏輯只寫一次：

1. **知識庫同步**：`RoutedConverter`（取代現有 `commands/knowledge_base.rs` 的 `MarkItDownConverter`）實作 `knowledge_base::ingest::DocumentConverter` trait，內部呼叫 `convert_document`
2. **手動轉檔工具**：新的 `document_convert` Tauri command（取代 `markitdown_convert`，`commands/markitdown.rs` 一併改名/搬移）直接呼叫 `convert_document`

anydoc 轉換本身是 CPU-bound 同步呼叫（`anydoc::to_markdown`），透過 `tokio::task::spawn_blocking` 包起來，避免大檔案的轉換時間卡住 async runtime。

MarkItDown 那一側（spawn python child process、串流 stdout/stderr、vision credential 解析）完全不動，`convert_document` 只是在原本 `markitdown_convert` 的邏輯前面加一層「這個副檔名要不要先試 anydoc」的判斷。

### 格式路由表

`engine_for_extension` 的對照表：

| 引擎 | 副檔名 |
|---|---|
| anydoc | `docx` `doc` `docm` `pptx` `ppt` `pptm` `ppsx` `ppsm` `pot` `pps` `xlsx` `xls` `xlsm` `xlsb` `odt` `ods` `odp` `rtf` `epub` `csv` `pdf` |
| MarkItDown | `html` `htm` `jpg` `jpeg` `png` `gif` `webp` `msg` `txt` `md` `rst` `xml` `json` `yaml` `yml` |

`knowledge_base/ingest.rs` 的 `SUPPORTED_EXTENSIONS` 與手動轉檔工具的原生檔案選擇器篩選器（原 `commands/markitdown.rs:176`，隨改名一起搬到 `document_convert` 模組）都依這張表擴充。這兩處目前是各自硬編的清單；實作時改為兩者都從 `document_convert` 模組匯出的同一個副檔名常數組出來（例如 `engine_for_extension` 涵蓋的所有副檔名聯集），不維持兩份獨立清單，避免日後新增格式時漏改其中一處。

### 設定欄位

`src-tauri/src/config/types.rs` 的 `AppConfig` 新增：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocConvertEngine {
    #[default]
    Auto,           // anydoc 優先，不支援的格式回退 MarkItDown
    MarkitdownOnly, // 完全不碰 anydoc，等同現有行為
}

// AppConfig 內：
#[serde(default)]
pub doc_convert_engine: DocConvertEngine,
```

前端比照 `submit_shortcut` 的做法（`GeneralPage.tsx`），加一組 radio group：

- 「自動（推薦）」— 説明文字：較快、轉換品質較好，不支援的格式仍會用 MarkItDown
- 「只用 MarkItDown」— 説明文字：維持原有行為

### Cargo 依賴

`src-tauri/Cargo.toml` 新增 `anydoc`（`cargo add anydoc`）。純 Rust crate，不需要 Python/uv，三平台原生編譯，不需要另外寫 `tauri.{macos,windows,linux}.conf.json` 平台分支，也不需要碰 `python_env` 模組。

## 錯誤處理

- **Auto 模式，anydoc 對它「該支援」的格式轉換失敗**（`anydoc::ConvertError` 的 `Encrypted`／`Malformed`／`Unsupported`／`ResourceLimit`／`MissingPart` 任一變體）：自動改用 MarkItDown 重試一次。
- **兩個引擎都失敗**：回傳合併錯誤訊息，讓使用者看得出兩邊都試過：
  `"anydoc: {anydoc 錯誤}；已改用 MarkItDown 重試但仍失敗：{markitdown 錯誤}"`
- **MarkitdownOnly 模式**：完全不呼叫 anydoc，行為與現在一致，失敗訊息不變。
- **anydoc 本來就不支援的格式**（圖片/音檔/msg/html/純文字）：兩種模式下都直接走 MarkItDown，`engine_for_extension` 查表決定，不會先嘗試 anydoc 再失敗。
- **成功時**（含 fallback 成功）不特別提示使用者用了哪個引擎，靜默即可。

## 測試

- **`engine_for_extension` 單元測試**：涵蓋路由表裡每一種副檔名，包含新增的 6 種格式與其變體，以及大小寫副檔名（`.DOCX` 等）
- **`convert_document` fallback 邏輯測試**：用假的 anydoc/MarkItDown 呼叫端（比照現有 `tests/knowledge_base_ingest.rs` 的 `FakeConverter`/`PanicOnFileConverter` 手法做依賴注入）驗證：
  - anydoc 失敗 → 自動改用 MarkItDown，最終成功
  - 兩者皆敗 → 合併錯誤訊息包含兩邊的錯誤內容
  - `MarkitdownOnly` 模式下 anydoc 完全不被呼叫
- **KB 同步整合測試**：`RoutedConverter` 實作 `DocumentConverter`，沿用現有 `sync_notebook` 測試骨架（`tests/knowledge_base_ingest.rs`）
- **anydoc 真實轉換的煙霧測試**：挑 1-2 個小型真實檔案（一份 `.docx`、一份 `.pdf`）跑過 `anydoc::to_markdown`，確認 crate 整合起來真的能動，不只是編譯過
- **前端**：`DocConverterView.test.tsx` 更新 import 路徑（`document_convert` 取代 `markitdown_convert`）；設定頁新增 radio group 的渲染/切換測試
