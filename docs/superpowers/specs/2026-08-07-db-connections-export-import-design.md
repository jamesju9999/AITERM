# 資料庫連線匯出／匯入 — 設計

日期：2026-08-07
狀態：已核可，待寫實作計畫

## 問題

資料庫連線只能一筆一筆手動建立。換電腦、重裝 app、或把一組連線交給同事時，使用者得對著舊機器的畫面重新打一遍主機、埠、資料庫名、帳號、密碼。連線一多（目前實際使用情境有六筆，橫跨 SQLite／MSSQL／DB2）這件事就變得很痛。

設定本身存在 `AppConfig.db_connections`（`src-tauri/src/config/types.rs:290`），密碼另存於 OS Keychain（key = `db:{id}`，見 `src-tauri/src/commands/db.rs:54`）。兩者分屬不同儲存體，使用者沒有辦法自己複製檔案搬走。

## 範圍

**含：** 資料庫連線的匯出與匯入，兩邊都提供勾選清單；匯出檔以 passphrase 加密。

**不含：**

- VCS 連線、MCP servers、AI 供應商、一般設定的匯出。格式設計上保留擴充空間（`format` 欄位有命名空間），但這一版不做。
- 雲端同步、自動備份。
- 匯出檔的版本升級路徑（目前只有 v1，v2 出現時再處理）。

## 檔案格式

副檔名 `.json`，預設檔名 `aiterm-db-connections.json`。

```json
{
  "format": "aiterm-db-export",
  "version": 1,
  "kdf": {
    "alg": "argon2id",
    "salt": "<base64, 16 bytes>",
    "m_cost": 19456,
    "t_cost": 2,
    "p_cost": 1
  },
  "cipher": "aes-256-gcm",
  "nonce": "<base64, 12 bytes>",
  "data": "<base64 ciphertext>"
}
```

明文部分只有格式識別與 KDF／cipher 參數，**不含任何連線資訊**。`data` 解密後是：

```json
{
  "connections": [
    {
      "id": "uuid",
      "name": "總行LBOTHODB",
      "db_type": "db2",
      "host": "172.19.2.83",
      "port": 25000,
      "database": "LBOTHODB",
      "username": "nuntio",
      "default_schema": "NUNTIO",
      "password": "..."
    }
  ]
}
```

### 為什麼是 JSON 信封而不是純二進位

明文 header 讓「這不是 AITerm 的匯出檔」和「passphrase 錯誤」變成兩種可分辨的錯誤，而不是籠統的「無法解密」。同時 `version` 在明文裡，可以在使用者輸入 passphrase **之前**就擋掉不支援的版本。

### 加密

- KDF：Argon2id，參數採 OWASP 建議（m=19 MiB, t=2, p=1）。salt 每次匯出重新隨機產生。
- 加密：AES-256-GCM，nonce 每次隨機產生。GCM 的驗證標籤同時涵蓋「passphrase 錯誤」與「檔案遭竄改」兩種情況——兩者都表現為解密失敗。
- 新增 crate：`aes-gcm`、`argon2`、`rand`。皆為純 Rust 實作，macOS／Windows／Linux 三平台皆可編譯，不引入 OpenSSL 依賴（與現有 `rustls-tls` 的取捨一致）。

匯出**一律**要求 passphrase，即使選取的連線全是無密碼的 SQLite 也一樣。行為單純可預期，勝過「有時要有時不要」。

### 版本規則

- `version == 1` → 繼續
- `version > 1` → 拒絕，顯示「此檔案由較新版本的 AITerm 匯出，請先更新 AITerm」，流程結束

高版本一律擋下，不做「盡力而為」的部分匯入。v1 的程式無法分辨 v2 是「只新增欄位」（安全）還是「改變既有欄位語意」（會安靜地匯入錯誤資料，使用者要到連線失敗才發現）。擋下來的代價只是請使用者更新 app，比較輕。

反之，日後版本必須保留讀取舊版檔案的能力——這是版本欄位存在的意義。

## 後端

加解密邏輯獨立成 `src-tauri/src/commands/db_export.rs`，對外只有兩個純函式，完全不依賴 Tauri，可單獨單元測試：

```rust
pub fn encrypt_payload(payload: &ExportPayload, passphrase: &str) -> anyhow::Result<Vec<u8>>
pub fn decrypt_payload(bytes: &[u8], passphrase: &str) -> Result<ExportPayload, ImportError>
```

`ImportError` 是列舉，對應下方錯誤表的各種情況，讓 UI 能給出精確訊息而非字串比對。

### 新增指令

| 指令 | 輸入 | 輸出 |
|---|---|---|
| `db_check_import_file` | `path` | `{ version: u32 }`，格式或版本不合則 Err |
| `db_export_connections` | `path, ids: Vec<String>, passphrase` | `()` |
| `db_preview_import` | `path, passphrase` | `Vec<ImportPreviewItem>` |
| `db_import_connections` | `path, passphrase, ids: Vec<String>` | `ImportResult` |

```rust
pub struct ImportPreviewItem {
    pub id: String,
    pub name: String,
    pub db_type: DbType,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub conflict: ConflictKind,      // New | Overwrite
    pub existing_name: Option<String>, // Overwrite 時填現有那筆的名稱
}

pub struct ImportResult {
    pub added: usize,
    pub overwritten: usize,
    pub failures: Vec<ImportFailure>, // { name, reason }
}
```

`ImportPreviewItem` **不含 password**。現有的 `DbConnectionInfo`（`src-tauri/src/commands/db.rs:42`）就從不外送密碼，這個設計維持同樣的界線：明文密碼只在 Rust 內部流動。前端在自己的 state 裡持有 passphrase，匯入時再送一次讓後端重新解密——代價是解密兩次，換取密碼不跨 IPC 邊界。

`db_export_connections` 從 `ConfigStore` 取設定、從 `SecretStore` 取密碼組成 payload。某筆連線在 Keychain 裡沒有密碼時（例如 SQLite），`password` 存空字串。

### 衝突判定

對匯出檔裡的每一筆，依序：

1. 現有連線中有相同 `id` → `Overwrite`，目標是該筆
2. 否則，現有連線中有相同 `name`（trim + 忽略大小寫）→ `Overwrite`，目標是該筆
3. 否則 → `New`

比對名稱是為了處理「同事在他的機器上手動建了同名連線」的情況——id 不同但實際是同一筆，不比名稱的話會變成兩筆同名連線並存。

### 套用規則

- `Overwrite`：沿用**現有那筆的 id**，更新其餘欄位；`password` 非空才寫 Keychain（空字串代表匯出時就沒有密碼，不該把現有密碼清掉）
- `New`：沿用**匯出檔裡的 id**。這讓同一份檔案重複匯入是冪等的——第二次匯入時 id 對得上，走 Overwrite 而不是再新增一筆

### 失敗處理

逐筆套用，不做全有全無。成功的留下，失敗的收進 `ImportResult.failures` 逐筆回報。`ConfigStore` 沒有交易語意，硬做 rollback 需要自行實作快照與還原，而還原本身也可能失敗——反而更容易寫壞。

## 前端

`src/components/Settings/DatabaseConnectionsPage.tsx` 標題列右側，在「+ 新增連線」左邊加入「匯出」「匯入」兩顆 `aiterm-btn--secondary`。連線清單為空時，「匯出」disabled。

### 匯出流程

1. 點「匯出」→ 面板內展開勾選清單（沿用現有連線卡片樣式，加上 checkbox，預設全勾）
2. 下方兩個 password 輸入框：passphrase、確認 passphrase
3. 兩者不符、任一為空、或一筆都沒勾時，「匯出」鈕 disabled
4. 按「匯出」→ `save()` 對話框（`@tauri-apps/plugin-dialog`，預設檔名 `aiterm-db-connections.json`，filter `JSON`）
5. 使用者取消對話框則不呼叫任何 IPC
6. invoke `db_export_connections` → 成功顯示「已匯出 N 筆連線」

### 匯入流程

```
選檔 → db_check_import_file ─┬─ 格式/版本不合 → 顯示錯誤，流程結束
                             └─ OK → 輸入 passphrase → db_preview_import → 勾選 → 匯入
```

1. 點「匯入」→ `open()` 對話框選檔；取消則不呼叫 IPC
2. invoke `db_check_import_file`。失敗直接顯示錯誤，**不要求輸入 passphrase**——使用者不該為一個注定被拒的檔案白打一次密碼
3. 通過後顯示 passphrase 輸入框 → invoke `db_preview_import`
4. 顯示勾選清單（預設全勾），每列右側標「新增」或「覆蓋（原：xxx）」
5. 按「匯入」→ invoke `db_import_connections` → `load()` 重整清單
6. 顯示「新增 N 筆、覆蓋 M 筆」；有 `failures` 時逐筆列出

### i18n

所有字串進 `src/lib/i18n.ts`，en 與 zh-TW 兩份齊備。

## 錯誤處理

| 情況 | 訊息（zh-TW） |
|---|---|
| 檔案不是合法 JSON，或 `format` 欄位不符 | 這不是 AITerm 的資料庫匯出檔 |
| `version` 高於支援 | 此檔案由較新版本的 AITerm 匯出，請先更新 AITerm |
| GCM 驗證失敗 | passphrase 錯誤，或檔案已損毀 |
| 檔案讀取／寫入失敗 | 顯示底層 IO 錯誤訊息 |
| 匯出時 Keychain 讀取失敗 | 該筆以空密碼匯出，並在完成訊息中提示哪幾筆沒帶到密碼 |
| 匯入時 Keychain 寫入失敗 | 該筆計入 `failures`，訊息為「設定已匯入，但密碼儲存失敗」 |

## 測試

### Rust（`src-tauri/tests/`）

- `encrypt_payload` → `decrypt_payload` round-trip 還原完整 payload
- 相同 payload 加密兩次產生不同密文（salt／nonce 確實隨機）
- 錯誤 passphrase → `ImportError::WrongPassphrase`
- 竄改 `data` 一個位元組 → 解密失敗（GCM 驗證標籤生效）
- `format` 欄位錯誤 → `ImportError::NotAnExportFile`
- `version: 2` → `ImportError::UnsupportedVersion`，且在解密之前就回傳（用一個 passphrase 絕對錯誤的檔案驗證：仍應得到版本錯誤而非 passphrase 錯誤）
- 衝突判定矩陣：id 中／name 中／都沒中／id 與 name 分別指向不同的兩筆（應以 id 為準）
- 空密碼的連線匯入時不覆寫現有 Keychain 密碼

### 前端（`DatabaseConnectionsPage.test.tsx`）

- 匯出面板預設全勾；取消勾選後只送出勾選的 id
- 兩次 passphrase 不符時「匯出」鈕 disabled
- 使用者取消 `save()`／`open()` 對話框時不呼叫任何 IPC
- `db_check_import_file` 失敗時顯示錯誤，且**不**顯示 passphrase 輸入框
- 匯入預覽正確渲染「新增」與「覆蓋（原：xxx）」標籤
- 匯入完成後呼叫 `load()` 重整清單

## 跨平台

- `aes-gcm`／`argon2`／`rand` 皆為純 Rust，無平台原生依賴
- 檔案路徑一律由 `plugin-dialog` 產生，不自行組字串
- Keychain 存取沿用現有 `SecretStore`，已支援三平台
