# AITerm CLI Host — 設計

日期：2026-09-10
狀態：待使用者複審

## 問題

AITerm 已經有完整的「主控端 ↔ 觀看端」遠端終端機機制，觀看端還能用自己的 AI
provider 跑 agent 迴圈去操作遠端的 shell（`RemoteAiPanel`）。但**主控端必須是一個
開著視窗的 AITerm GUI 分頁**——這讓整套機制用不到最想用的地方：沒有桌面環境的雲端
伺服器、容器、NAS。

要的東西是一個 headless 的 CLI 執行檔：丟到那台機器上跑起來，桌面版 AITerm 連進去，
就能用 AITerm 裡的 AI 去操作它。

## 現況調查（讀過程式碼確認，非推測）

### 已經做好、不需要重做的部分

- `src-tauri/src/share/`（2497 行）已經是完整的主控端實作：axum WS server over TLS、
  6 位短碼配對、SAS 承諾流程防中間人、mDNS 廣播、觀看者名單與控制權授予/收回。
- `share/protocol.rs:68` `PROTOCOL_VERSION = 2`。文字 frame 是 JSON 控制訊息，二進位
  frame 直接是 PTY 位元組（不套 base64）。
- 觀看端 `src/ipc/shareViewer.ts:31` 的 `share_viewer_connect(host, port, code,
  displayName)` **已經吃手動位址與埠**，不依賴 mDNS 發現。
- `src/components/RemoteTerminalView/RemoteAiPanel.tsx` 已經是完整的遠端 AI 面板：
  多輪對話、model picker、串流、對話紀錄。AI 跑在**觀看端**，用觀看端自己的 provider。
- 遠端 AI agent 迴圈偵測「這一步跑完了」靠的是主控端 shell 送出的 `OSC 133;D`
  （含 exit code）。而 `pty/shell.rs` 的注入邏輯**完全不依賴 Tauri**——Windows 的
  PowerShell 腳本走 `dirs::data_local_dir()`（`shell.rs:49`），不是 Tauri resource
  dir。所以 CLI host 起的 shell 自動就有這個標記。

### Tauri 耦合的實際深度（比預期淺）

| 檔案 | Tauri 參照 |
|------|-----------|
| `share/registry.rs`、`share/tls.rs`、`share/mdns.rs` | **0 個** |
| `pty/session.rs`、`pty/shell.rs`、`pty/ansi.rs`、`pty/cd_parser.rs` | **0 個**（grep 命中只有測試字串裡的 `src-tauri` 路徑） |
| `share/server.rs` | `Option<AppHandle>` 兩個欄位（:46、:52）+ 兩處 `emit`（:193-194 推播 pending request、:418-419 推播 viewers-changed） |
| `share/mod.rs` | 兩個 `Option<tauri::AppHandle>` 參數（:82、:95），只是往下傳 |
| `pty/manager.rs` | 只有 `create_with_app`（:29）用 `AppHandle` + `Emitter`；`create_with_callback`（:64）本來就無 Tauri 依賴 |
| `share/viewer_manager.rs`、`pty/commands.rs` | emit 密集／`#[tauri::command]`——這些是觀看端與 GUI 專屬，本來就不該進 core |

### TLS 與 exporter

- `share/viewer.rs:22-27` 的憑證驗證器**接受任何憑證**。自簽憑證沒有憑證鏈可驗，
  身分保證完全來自 SAS 人工核對。這是刻意的設計，不是偷懶。
- `share/tls.rs:96` `exporter_material(conn)` 用 RFC 5705 keying material exporter，
  label 是 `SAS_EXPORTER_LABEL`（`tls.rs:29`），長度 32（`tls.rs:32`）。
  **同一條 TLS 連線的兩端會導出相同的值**，`server.rs:175` 與 `viewer.rs:163` 各自
  算出自己那份 SAS 就是靠這個性質。

### 版本檢查是嚴格相等

`server.rs:139` 是 `if protocol_version != PROTOCOL_VERSION`。原始碼註解明講「同事的
AITerm 沒更新在區網分享裡是常態，不是邊角案例」。**這代表把 `PROTOCOL_VERSION` 升到
3 會讓 v1.24 的使用者連不上 v1.25 的同事**——是本設計最重要的約束。

`protocol.rs` 的 `an_unknown_server_message_fails_to_parse_rather_than_being_ignored`
測試已經證明：serde 對未知的 enum **tag** 回 `Err` 而不是忽略。所以新增
`EndReason` 變體同樣會讓舊觀看端硬性解析失敗。

### 短碼與生命週期

`registry.rs:81` `start_share(tab_id)` 產生 6 位短碼，分享期間一直有效
（`stop_share` 才作廢）。所以「觀看端斷線後重連」本來就成立。

### 建置現況

- `src-tauri/Cargo.toml`：單一 package `app`，lib 名 `aiterm_lib`，**沒有 workspace**。
- `src-tauri/src/main.rs`：`--headless` **已經被佔用**——是 `enterprise::headless`，
  一個向企業伺服器拉取 task packet 的 worker（pull 模式），跟本設計無關。不要重用
  這個旗標。
- `.github/workflows/release.yml`：`v*` tag 觸發，六個矩陣目標（mac aarch64、windows
  x64、linux gnu x64/arm64 各出 AppImage 與 deb）。**沒有 musl，也沒有
  `x86_64-apple-darwin`**。
- repo 是 public（`github.com/jamesju9999/AITERM`）。
- `build.rs`（`tauri-build`）在編譯期驗證每個 `externalBin` 存在於磁碟——這個檢查
  綁在 `app` package 上，所以獨立的 CLI package **不需要 uv／db2 sidecar 就能編**。

## 範圍（brainstorming 已確認的決定）

1. **連線方向維持現狀**：CLI host 監聽、GUI 觀看端主動連。跨網段交給使用者的網路
   （Tailscale／WireGuard／SSH tunnel／防火牆規則）。不做 relay、不做反向連線。
2. **單一 session**：一個 CLI host 行程 = 一個 shell。要多個就跑多個行程，或在裡面
   自己開 tmux。協定不動。
3. **金鑰就是身分，CLI 模式拿掉短碼**：金鑰寫在檔案裡，重啟不變；埠可固定。GUI 裡
   存的那筆連線永遠有效。
4. **認證＝預共享金鑰 + HMAC 綁 TLS exporter，雙向互證**。金鑰不上線。
5. **抽出 `aiterm-core` crate，CLI 是獨立 bin**，零 GUI 依賴。
6. **真搬移**（`git mv`），不複製、不用 `#[path]` hack。兩邊永遠共用同一份協定原始碼。
7. **CLI host 不需要任何 AI 設定或 API key**。AI 跑在觀看端。
8. **四條發布管道**：GitHub Releases + 安裝腳本（含 musl 靜態）、ghcr.io 容器映像、
   Homebrew tap、npm/npx。

## 架構

### Workspace 佈局

**Workspace 根放在 `src-tauri/Cargo.toml`，不放 repo 根目錄。** `app` package 自己
兼任 workspace root。

```
src-tauri/
├── Cargo.toml              [package] app + [workspace] members = [".", "crates/*"]
├── crates/
│   ├── aiterm-core/        零 Tauri 依賴
│   └── aiterm-host/        CLI bin，只依賴 aiterm-core
├── src/                    app crate（GUI）
└── target/                 位置不變
```

理由很具體：workspace 根若放 repo 根目錄，`target/` 會整個搬到 repo 根——worktree
是靠 symlink `src-tauri/target` 共用的（那是數十 GB 的重編成本），CI 路徑與
`tauri-build` 也都得跟著改。放在 `src-tauri/` 底下則全部不動。

### 模組歸屬

搬進 `aiterm-core`：

| 模組 | 改動 |
|------|------|
| `pty::{session, shell, ansi, cd_parser, error, events}` | 原封搬移，零行為改動 |
| `appimage_env` | 原封搬移。實作時才發現的相依：`pty::session` 在 spawn 子行程時呼叫 `crate::appimage_env::appimage_env_fixes()`。它本身零 Tauri 參照，而且「修正 PTY 子行程的環境變數」正是 core 的職責；不搬的話就得改 `session.rs` 裡的路徑，那會破壞純搬移 |
| `pty::manager` | 搬移；`create_with_app` 留在 app crate |
| `share::{protocol, registry, tls, mdns}` | 原封搬移 |
| `share::server`、`share::mod` | 搬移 + 事件抽象 |
| `share::auth` | **新增** |

留在 app crate：`pty::commands`、`share::viewer_manager`、`commands/share*.rs`，
以及一個薄的 `create_with_app`——它做的三件事（產生 id、包一個 emit closure、注入
bridge 環境變數）全是 GUI 專屬，而核心的 `create_with_callback` 本來就存在且無
Tauri 依賴。app crate 用 `pub use aiterm_core::{pty, share}` 讓既有的
`crate::pty::…` / `crate::share::…` 路徑不變。

### 事件抽象

`share::server` 現在拿 `Option<AppHandle>` 做兩件事：推播「有人要連進來」、推播
「觀看者名單變了」。換成：

```rust
pub trait ShareEvents: Send + Sync + 'static {
    fn pending_request(&self, ev: &PendingRequestEvent);
    fn viewers_changed(&self);
}
```

GUI 端實作成 `emit`；CLI 端實作成明確的 no-op 型別（`struct SilentEvents`）。
維持 `Option` 語意的話 CLI 端連實作都不用寫，但明確的 no-op 讓「CLI 模式下這些事件
去哪了」在程式碼裡看得見。

### 綁定位址要可指定

**埠已經可以指定了**——`share/mod.rs` 早就有 `start_if_needed_on_port(pty, port, app)`，
為了手動的區網連通性檢查而加。`start_if_needed` 只是 `port = 0` 的包裝。所以 CLI 的
`--port` 直接用既有的那支就好。

真正寫死的是**位址**：`SocketAddr::from(([0, 0, 0, 0], port))`。要支援 `--bind` 就把
它提升成參數，GUI 呼叫端傳 `0.0.0.0` 維持現行行為。

### 一個合成的 tab_id

`ShareRegistry` 是以 `tab_id` 為索引的（`registry.rs:81` 的 `start_share(tab_id)`）。
CLI host 只有一個 session，啟動時用一個固定的合成 id（例如 `"cli"`）呼叫一次
`start_share`；它回傳的 6 位短碼在 CLI 模式**不印出、不使用**。這樣 registry 的
viewer／控制權管理邏輯完全不用改。

### 核准的接點

短碼模式下，`server.rs` 的流程送出 `AwaitingApproval` 之後會輪詢 registry 等
`share_approve` 把裁決寫進去。CLI 模式下的接點就在這裡：驗過 `auth` 之後**立刻**
呼叫 `registry.approve(request_id, mode)`（`mode` 由 `--read-only` 決定），流程往下
走的路徑與短碼模式完全相同。這是整個 CLI host 唯一改變核准語意的地方。

## 認證握手

### 訊息序列一個字都不動

維持 `SasCommit → Join → AwaitingApproval → Granted`。**不升 `PROTOCOL_VERSION`**
（理由見上面的現況調查），只加兩個可選的附加欄位，核准來源換人。

```rust
// protocol.rs
ClientMessage::Join {
    protocol_version: u32,
    code: String,
    display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth: Option<String>,          // 新增
}

ServerMessage::Granted {
    mode: WireAccessMode,
    cols: u16,
    rows: u16,
    host_os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_auth: Option<String>,     // 新增
}
```

`skip_serializing_if` 讓短碼模式送出的 JSON 與現在**逐位元組相同**，舊版收到的東西
不變。

### 流程

1. 觀看端送
   `Join { protocol_version: PROTOCOL_VERSION, code: "", display_name, auth: Some(proof_viewer) }`
   （`PROTOCOL_VERSION` 維持 2，觀看端一律送常數，不寫死數字）
   其中 `proof_viewer = hex(HMAC-SHA256(K, b"aiterm-viewer-v1" ‖ exporter_auth))`
2. CLI host 用**自己那條 TLS 連線**導出的 `exporter_auth` 重算，用 constant-time
   比較。不符 → `Ended { EndReason::Denied }`。
3. CLI host 回
   `Granted { …, host_auth: Some(hex(HMAC(K, b"aiterm-host-v1" ‖ exporter_auth))) }`，
   mode 依 `--read-only` 決定。
4. 觀看端**驗過 `host_auth` 才渲染畫面、才送出任何按鍵**。

### 為什麼一定要雙向

只驗觀看端的話，中間人雖然偽造不出 `proof_viewer`，但它可以乾脆自己扮演 host——
直接回 `Granted`、餵假畫面、收走你打的每一個鍵。觀看端不驗憑證（`viewer.rs:22-27`），
所以「對面真的握有金鑰」必須由 `host_auth` 提供。

兩個方向用不同的 label（`aiterm-viewer-v1` / `aiterm-host-v1`）是為了擋反射攻擊：
把一邊的證明原封送到另一邊要不成立。

### 為什麼不新增 EndReason 變體

`Denied` 是既有變體，v2 觀看端解得開。新變體會讓舊觀看端在 `serde_json::from_str`
硬性失敗，變成「無法解釋的斷線」——那正是 `protocol.rs` 已有測試在防的事。語意上
「主控端拒絕」也不算誤導。

### exporter 用獨立 label

`tls.rs` 新增

```rust
pub const AUTH_EXPORTER_LABEL: &[u8] = b"EXPERIMENTAL aiterm cli-host auth v1";
```

並把 `exporter_material` 重構成吃 label 的版本（`exporter_material_with_label`），
`SAS_EXPORTER_LABEL` 的既有呼叫端行為不變。**不重用 SAS 的 material**：同一份秘密
同時餵給兩個不同用途的建構，是跨協定攻擊的標準溫床。

### 空的 code

CLI 模式送 `code: ""`，CLI host 忽略。副作用是好的：拿金鑰模式的連線去指一台 GUI
host，會在 `registry.rs:111` 的 `tab_for_code("")` 乾淨地失敗成 `InvalidCode`。

### 金鑰儲存

- `--key-file`，預設 `$XDG_CONFIG_HOME/aiterm-host/key`，退回 `~/.config/aiterm-host/key`。
  Windows 用 `dirs::config_dir()`。
- 首次執行產生 32 bytes 隨機（`rand`，已是直接依賴），以 0600 建檔。
- 也吃 `AITERM_HOST_KEY` 環境變數（容器情境）。兩者都給時環境變數優先，並印出警告。
- 檔案權限比 0600 寬 → **拒絕啟動**，比照 ssh。Windows 不做這個檢查（ACL 語意不同），
  改為在啟動訊息裡提示金鑰檔位置。

## CLI 介面

```
aiterm-host [OPTIONS]
  --port <PORT>        監聽埠（預設 8022。GUI 是隨機埠，CLI 必須可固定）
  --bind <ADDR>        預設 0.0.0.0
  --key-file <PATH>
  --shell <PATH>       預設沿用 pty::shell 既有的偵測
  --cwd <PATH>
  --read-only          連進來的人只能看
  --advertise          開啟 mDNS 廣播（預設關）
  --print-connection   只印連線資訊就退出
```

**刻意沒有 `--name`。** 協定裡沒有任何欄位可以把主機名稱送給觀看端（`Granted`
只帶 mode／尺寸／`host_os`），加一個就是又一次協定擴充。連線在 GUI 那邊由使用者
自己命名即可。

**mDNS 預設關閉，跟 GUI 相反。** 伺服器情境用不到，而且那等於在辦公室網路上廣播
「這裡有一個 shell」。

啟動時往 stdout 印：綁定位址、埠、金鑰、以及一行可直接貼進 GUI 的連線字串。

### 生命週期

- shell 結束 → 行程以該 shell 的 exit code 退出。
- SIGTERM / SIGINT（Windows：Ctrl+C handler）→ 對所有觀看端送
  `Ended { HostStoppedSharing }`、收掉 PTY、乾淨退出。
- 不自動重開 shell。重啟交給 systemd / docker restart；金鑰與埠都固定，所以 GUI 裡
  存的那筆連線在重啟後依然有效。

## 觀看端（GUI）改動

範圍刻意很小：

- `share_viewer_connect` 加 `key: Option<String>` 參數（`src/ipc/shareViewer.ts:31`
  與對應的 `commands/share_viewer.rs`）。
- `share::viewer` 的握手：`key` 是 `Some` 時計算並帶上 `auth`、驗 `host_auth`、
  **不顯示 SAS 也不等使用者**；`None` 時走現有的短碼 + SAS 流程，一行都不變。
- 連線對話框加一個「金鑰」欄位。填了走金鑰模式，留空走短碼模式。
- 金鑰存進既有的 `SecretStore`（OS keyring），**不進 localStorage**。
- `RemoteTerminalView` / `RemoteAiPanel` 不動。

## 錯誤處理

| 情況 | 行為 |
|------|------|
| 金鑰檔權限過寬（Unix） | 拒絕啟動，訊息指出檔案路徑與應有的 0600 |
| 埠被佔用 | 明確訊息，含埠號與 `--port` 提示 |
| 找不到可用 shell | 明確訊息（對應 `PtyError::NoShellAvailable`） |
| 認證失敗 | host 記 log（來源 IP + 自報名稱），對該來源 IP 做指數退避 |
| 觀看端驗 `host_auth` 失敗 | 立刻斷線，前端顯示「主機金鑰不符」，**不渲染任何收到的位元組** |
| 觀看端連 CLI host 卻沒帶金鑰 | 收到 `Ended { Denied }`，前端沿用既有的「主控端拒絕」文案 |

退避不是為了防爆破（256-bit 金鑰爆破不現實），是為了擋 log flooding 與握手階段的
資源耗用——這個埠會被丟在公開網路上。

## 測試

### 搬移的證據

搬過去的既有測試**必須在 `aiterm-core` 裡照樣全綠，且搬移那個 commit 不該有任何
測試需要修改**。這是「真搬移沒改行為」唯一有力的證據；如果有測試需要改，代表那不是
搬移，要停下來說清楚改了什麼。

### 新測試

- **協定相容性（最優先，見待驗證項）**：不帶 `auth` 的 v2 `Join` JSON 仍可解析；
  帶 `auth` 的 JSON 丟給不認得該欄位的形狀也能解析；短碼模式序列化出來的 JSON
  不含 `auth` 鍵。
- **auth 單元測試**：正確證明通過；錯金鑰被拒；把 `host_auth` 當 `auth` 送（反射）
  被拒；用另一條連線的 exporter 算出的證明被拒。每一條都要先確認在修正前會紅。
- **整合測試**：起一個真的 `aiterm-host` 行程，用既有的 `share::viewer` 客戶端連
  進去，跑一個指令，驗證輸出裡出現 `OSC 133;D`。這一條同時證明遠端 AI agent 迴圈的
  硬性前提在 CLI host 上成立。
- **CI**：加 musl 建置 job，且**要真的執行一次產出的二進位**（`--print-connection`），
  不是只編過就算。

## 發布管道

四條管道全部從**同一批 release artifact** 衍生，由 Action 自動 bump，彼此獨立——
哪一條卡住都不擋主線。

### 目標三元組

| 目標 | 用途 |
|------|------|
| `aarch64-apple-darwin` | Apple Silicon Mac |
| `x86_64-apple-darwin` | Intel Mac（現有矩陣沒有，CLI 補上） |
| `x86_64-unknown-linux-musl` | 靜態，alpine／distroless／老 glibc 伺服器 |
| `aarch64-unknown-linux-musl` | 同上，arm64 |
| `x86_64-pc-windows-msvc` | Windows |

musl 之所以可行：TLS 走 rustls（沒有 OpenSSL）、mDNS 是純 Rust 實作。
**但 `portable-pty` 在 musl 上的行為未經實測，列為待驗證項。**

### 管道

1. **GitHub Releases**：每個目標一個 `.tar.gz`（Windows `.zip`）+ 一份 `SHA256SUMS`。
   掛在既有的 `release.yml` 上，多一個 job 矩陣，不動現有的六個。
2. **安裝腳本**：`install.sh` 偵測 OS/arch → 抓對應 asset → 驗 checksum → 裝進
   `~/.local/bin`。Windows 給對應的 `install.ps1`。腳本從 release asset 提供，
   `curl -fsSL … | sh`。
3. **ghcr.io 容器映像**：基於 musl 靜態執行檔，Dockerfile 就是 `alpine` 加一個檔案。
   同 repo、public、免費。tag 跟版本走，另有 `latest`。
4. **Homebrew tap**：另開 `jamesju9999/homebrew-tap`，formula 的 URL/sha256 由
   release Action 自動 bump。macOS 與 Linuxbrew 通吃。
5. **npm**：走 esbuild 那套——一個入口套件 + 每個平台一個 optionalDependencies 子
   套件，各自只帶自己的執行檔。使用者 `npx` 零安裝。

### 待確認的外部資源

npm 上的 `aiterm` 名稱與 `@aiterm` scope 是否可用，要在動工前查。若被佔用就換名，
這只影響 npm 那一條，不影響其他管道。

## 明確排除

- relay 中繼伺服器、反向連線
- 一個 host 多個 session（tmux 式的列表／切換）
- host 端跑 AI（AI 永遠在觀看端）
- 工作看板派工到遠端主機
- GUI host 也支援金鑰模式（GUI 維持短碼 + SAS）
- 把主機名稱送給觀看端（需要再一次協定擴充，見 CLI 介面那節）
- Linux distro 套件（deb/rpm apt repo）、WinGet、AUR、nixpkgs、`cargo install`

## 待驗證項（實作前必須先做掉）

1. **serde 對 internally-tagged enum 的 struct variant，未知欄位是忽略還是報錯？**
   整個「不升 `PROTOCOL_VERSION`」的相容性策略建立在「忽略」之上。這跟 repo 已證明
   的「未知 *tag* 會報錯」是不同的機制，沒有實測過。
   驗證方式：`cargo test -p aiterm-core protocol`，測試內容見「新測試」第一項。
   **若結論是報錯**，相容性策略要改成升版本到 3，並接受 GUI 之間的跨版本分享會斷；
   那是需要回頭找使用者確認的範圍變更，不是實作者可以自己決定的事。
2. **`portable-pty` 在 `*-unknown-linux-musl` 上能不能建置並實際開出 PTY？**
   驗證方式：在 CI 或本機用 cross／docker 建一次 musl 目標，把產出的二進位丟進
   `alpine` 容器跑起來、開一個 shell、確認收得到輸出。
   **若不行**，musl 那兩格改成 gnu 靜態或降級為動態連結，容器映像基底從 `alpine`
   換成 `debian-slim`。這只影響發布管道，不影響核心設計。
3. **npm `aiterm` / `@aiterm` 是否可用**（見上）。
