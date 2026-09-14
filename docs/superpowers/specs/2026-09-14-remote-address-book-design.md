# 遠端終端機地址簿 — 設計

日期：2026-09-14
狀態：已核准，待實作

## 問題

連線到 `aiterm-host` 每次都要手動貼三樣東西：位址、埠、64 個十六進位字元的金鑰。
金鑰在主機端是長期有效的（存在 `~/.config/aiterm-host/key`，重開機不變），
但觀看端完全不記，所以每一次連線都要重貼一次同一串東西。

## 先釐清一件事：金鑰持久化不是障礙

`src-tauri/src/commands/secret.rs` 的註解寫著「never expose a raw `set_api_key`
command」。那句話的範圍是**泛用的寫入指令**，不是「不能存密碼」——這個 repo 已經
有三個功能各自把密碼寫進 keyring，都用具名的、帶型別的指令：

| 功能 | 寫入點 | keyring 的 key |
|---|---|---|
| 資料庫連線 | `commands/db.rs:213` | `secret_key(&id)` |
| VCS 連線 | `commands/vcs.rs:196` | `vcs:{id}` |
| 郵件帳號 | `commands/mail.rs:75` | `mail_secret_key(&id)` |

地址簿照同一個形狀做，不需要改動那個既有決策。

## 現況

`src/components/ConnectDialog/index.tsx`（192 行）有兩種連線模式：

| 模式 | 欄位 | 金鑰是否固定 |
|---|---|---|
| 短碼 | 6 位短碼（＋mDNS 自動搜尋） | 不適用，碼每次都不同 |
| 金鑰 | 位址 ＋ 金鑰（短碼留空） | **是**，主機重開仍有效 |

地址簿只服務金鑰模式。短碼模式的碼每次都不一樣，存起來沒有意義。

後端 `share_viewer_connect`（`commands/share_viewer.rs:13`）目前的簽章：

```rust
host: String, port: u16, code: String, display_name: String, key: Option<String>
```

`key` 由前端傳入。

## 設計

### 1. 資料模型

`AppConfig` 新增一個欄位，跟 `vcs_connections` 完全同一個姿勢
（`#[serde(default)]`，舊設定檔載入時是空陣列，不需要遷移程式碼）。
設定檔是 **TOML**（`config/mod.rs:184` 的 `config.toml`），不是 JSON：

```rust
/// Saved remote terminal hosts (keys stored separately in Keychain).
#[serde(default)]
pub remote_hosts: Vec<RemoteHost>,
```

```rust
pub struct RemoteHost {
    pub id: String,     // uuid
    pub name: String,   // 使用者取的別名
    pub host: String,
    pub port: u16,
}
```

**金鑰不在這個結構裡**，存在 keyring，key 為 `remote:{id}`。

### 2. 金鑰不跨 IPC（核心決策）

地址簿最自然的錯誤做法是加一個「讀出某筆的金鑰」指令讓前端拿去傳給
`share_viewer_connect`。那等於在 IPC 上開了一個讀出任意已存金鑰的介面——
正是 `secret.rs` 的註解在防的東西。

改成：`share_viewer_connect` 多一個參數。

```rust
pub async fn share_viewer_connect(
    host: String,
    port: u16,
    code: String,
    display_name: String,
    key: Option<String>,
    saved_host_id: Option<String>,   // 新增
    viewers: State<'_, Arc<ViewerManager>>,
    secrets: State<'_, Arc<SecretStore>>,  // 新增
    app: AppHandle,
) -> Result<Connected, String>
```

規則：

- `saved_host_id` 有值時，後端自己去 keyring 讀 `remote:{id}`，**忽略前端傳來的
  `key`**（前端在這條路上根本不會填它）。
- keyring 取不到時**回一個可辨識的錯誤**，不要退回 `None` 往下走——往下走會被
  當成短碼模式，錯誤訊息會指向短碼不符，跟真正的原因毫無關係。
  `ConnectDialog` 現有註解記錄的 `Some("")` 陷阱就是同一類問題。
- `saved_host_id` 為 `None` 時行為完全不變（手動與短碼兩條既有路徑不受影響）。

金鑰因此從頭到尾不跨 IPC，跟 `db.rs`／`vcs.rs`（連線在後端建立、密碼不回前端）
同一個姿勢。

### 3. 後端指令

新檔 `src-tauri/src/commands/remote_hosts.rs`，四個指令，照 `vcs.rs` 的形狀：

| 指令 | 行為 |
|---|---|
| `remote_hosts_list() -> Vec<RemoteHost>` | 只回設定檔內容，**不含金鑰** |
| `remote_hosts_add(input) -> String` | 產 uuid、寫設定檔、金鑰寫 `remote:{id}` |
| `remote_hosts_update(input)` | 更新設定檔；`secret` 有值且非空才覆寫金鑰 |
| `remote_hosts_remove(id)` | 設定檔移除 **並且** 刪掉 `remote:{id}` |

`ConfigStore` 對應新增 `add_remote_host` / `update_remote_host` / `remove_remote_host`，
與既有的 `add_vcs_connection` 等三個並列。

### 4. 前端

**元件切分。** `ConnectDialog/index.tsx` 目前 192 行，把清單塞進去會讓它同時負責
「模式切換」「搜尋」「清單管理」「存檔提示」四件事。抽出：

```
src/components/ConnectDialog/RemoteHostList.tsx   清單、每列的連線／編輯／刪除
```

`index.tsx` 保留模式切換與送出。

**清單。** 對話框最上方，有條目時才顯示。每列是「別名 — host:port」，點整列直接
連線（不打任何字），右側有編輯與刪除。

**存檔提示。** 連線成功之後、開分頁之前，在對話框裡就地顯示一行：

> 已連上。要把這台存進地址簿嗎？〔別名 ____〕〔儲存〕〔不用〕

按任一個才呼叫 `onConnected` 開分頁。連線在按之前就已經建立，所以這一下延後的是
開分頁、不是連線本身。刻意**不**用另一個 modal——這個 repo 踩過原生對話框在掛載
effect 裡被 StrictMode 呼叫兩次的坑，而且在 dialog 上疊 dialog 本來就難用。

出現條件：**只有手動輸入的金鑰模式**。短碼模式沒有固定金鑰可存；從地址簿點進來
的連線已經存過了，不再問。

**金鑰遺失。** 選了某一筆但後端回報 keyring 取不到金鑰（換了台電腦、keychain 被
清、設定檔被同步過去）時，顯示「這台的金鑰不在這台電腦上」並自動展開手動欄位、
帶入已知的位址與埠，讓使用者只需重貼金鑰。

### 5. i18n

新字串同時加進 `src/lib/i18n.ts` 的 en 與 zh-TW：清單標題、空清單提示、
存檔提示的三段文字、刪除確認、金鑰遺失的說明。

### 6. 測試

**Rust**

- `remote_hosts_add` 之後 `remote_hosts_list` 讀得到，且回傳值**不含金鑰**
- `remote_hosts_update` 不帶 secret 時不會清掉既有金鑰
- `remote_hosts_remove` 之後 keyring 裡的 `remote:{id}` 確實消失
- `share_viewer_connect` 帶 `saved_host_id` 時，真的從 keyring 取到金鑰
  （用一個存了金鑰的假 id 與一個沒存的假 id 做對照，證明測試會分勝負）
- `saved_host_id` 指向不存在的金鑰時回可辨識的錯誤，**不是**靜默退回 `None`
- `AppConfig` 少了 `remote_hosts` 欄位的舊設定檔仍能載入（`#[serde(default)]`）

**前端**

- 有條目時清單渲染得出來；沒有條目時整塊不顯示
- 點一列直接呼叫 `shareViewerConnect` 且帶 `savedHostId`、不帶 `key`
- 存檔提示只在手動金鑰模式的成功連線後出現
- 短碼模式成功連線後**不**出現存檔提示
- 從地址簿點進來的成功連線**不**出現存檔提示
- 按「不用」仍然會開分頁（不能因為不存就把連線丟掉）

## 明確不做

- 不做設定頁的獨立管理頁面。管理（改名、刪除、換金鑰）全部在連線對話框內完成。
- 不存短碼模式的條目。
- 不做匯入／匯出、不做跨機器同步。
- 不動 mDNS 搜尋那條路徑。
- 不改 `secret.rs` 的既有決策，不新增泛用的金鑰寫入或讀出指令。
