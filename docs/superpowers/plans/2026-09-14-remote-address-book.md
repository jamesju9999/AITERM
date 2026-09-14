# 遠端終端機地址簿 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓使用者把 `aiterm-host` 的位址、埠、金鑰、別名存起來，之後在連線對話框
點一筆就連上，不必重貼 64 個字元的金鑰。

**Architecture:** 紀錄存 `ConfigStore`（**TOML**，`#[serde(default)]`），金鑰存 OS keyring
（key 為 `remote:{id}`），照 `commands/vcs.rs` 走過的形狀。**金鑰不跨 IPC**：
`share_viewer_connect` 新增 `saved_host_id`，後端自己去 keyring 取。

**Tech Stack:** Rust（Tauri command、`keyring`、`uuid`）、React 19 + TypeScript、
Vitest + React Testing Library。

**Spec:** `docs/superpowers/specs/2026-09-14-remote-address-book-design.md`

---

## 測試環境的硬限制（動手前一定要讀）

`src-tauri/src/secret/mod.rs:199` 的註解寫著：碰真實 keychain 的測試一律標
`#[ignore]`，因為 CI 環境不一定有 keychain。這條限制決定了本計畫的切分：

**「`saved_host_id` 有值但 keyring 取不到金鑰時要報錯、絕不退回 `None`」是這個功能
最重要的一條規則**（退回 `None` 會被當成短碼模式，錯誤訊息指向短碼不符，跟真正的
原因毫無關係）。它**不能**只活在 `#[tauri::command]` 裡，否則永遠測不到。

所以 Task 2 把它抽成一個不碰 keyring 的純函式 `resolve_connect_key`，
查表動作用 closure 注入。Task 4 的 command 只負責把 `secrets.get` 接上去。

同理：`cargo test` 一定要加 `--workspace`（見 CLAUDE.md），否則只跑 `app`。

## 三個容易寫錯的既有慣例（已查證，照做就好）

1. **設定檔是 TOML，不是 JSON。** `config/mod.rs:184` 是 `config.toml`，
   讀寫走 `toml::from_str` / `toml::to_string_pretty`。測試建 store 一律
   `ConfigStore::new_at(dir.path().join("config.toml"))`（`db.rs:511` 就是這樣）。
2. **設定檔型別不加 `#[serde(rename_all = "camelCase")]`。** `VcsConnection`
   （`types.rs:514`）只有 `#[derive(Debug, Clone, Serialize, Deserialize)]`，
   欄位維持 snake_case。命令層的 `VcsConnectionInput` / `VcsConnectionInfo` 也一樣。
3. **前端刻意保留 snake_case 欄位名。** `src/ipc/vcs.ts:13` 是 `has_secret`、
   `vcs_type`，不是 `hasSecret`。所以本計畫的 TypeScript 用 **`has_key`**。
   （指令的**參數名**是另一回事——Tauri 會把 `displayName` 轉成 `display_name`，
   所以 `savedHostId` 這個參數名是對的。）

---

## 檔案結構

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/config/types.rs` | 新增 `RemoteHost` 型別與 `AppConfig.remote_hosts` 欄位 |
| `src-tauri/src/config/mod.rs` | 新增 `add/update/remove_remote_host` 三個存取方法 |
| `src-tauri/src/commands/remote_hosts.rs`（新增） | 四個 Tauri 指令 + `remote_host_secret_key` + **`resolve_connect_key` 純函式**與它的測試 |
| `src-tauri/src/commands/mod.rs` | 掛上新模組 |
| `src-tauri/src/commands/share_viewer.rs` | `share_viewer_connect` 新增 `saved_host_id` 與 `secrets` |
| `src-tauri/src/lib.rs` | 註冊四個新指令 |
| `src/ipc/remoteHosts.ts`（新增） | 四個指令的前端包裝 |
| `src/ipc/shareViewer.ts` | `ShareViewerConnectArgs` 新增 `savedHostId` |
| `src/components/ConnectDialog/RemoteHostList.tsx`（新增） | 清單與每列的連線／編輯／刪除 |
| `src/components/ConnectDialog/index.tsx` | 接上清單、連線成功後的存檔提示 |
| `src/lib/i18n.ts` | en 與 zh-TW 的新字串 |

---

## Task 1: 設定檔的資料模型

**Files:**
- Modify: `src-tauri/src/config/types.rs`
- Modify: `src-tauri/src/config/mod.rs`

- [ ] **Step 1: 先寫會紅的測試**

在 `src-tauri/src/config/types.rs` 最底下的 `mod tests` 裡加入
（旁邊就有 `app_config_has_vcs_connections_default`，照它的形狀）：

```rust
    #[test]
    fn app_config_has_remote_hosts_default() {
        let cfg = AppConfig::default();
        assert!(cfg.remote_hosts.is_empty());
    }

    #[test]
    fn a_config_without_remote_hosts_still_loads() {
        // 舊版設定檔沒有這個欄位。少了 #[serde(default)] 的話整份設定會解析
        // 失敗，使用者的所有設定一次全部消失——症狀跟「地址簿」完全無關。
        //
        // 用「序列化一份預設設定再讀回來」而不是手寫 TOML：手寫的字串很容易
        // 因為漏掉某個沒有預設值的欄位而變成在測別的東西。
        let serialized = toml::to_string_pretty(&AppConfig::default()).unwrap();
        assert!(
            !serialized.contains("remote_hosts"),
            "空的 remote_hosts 不該被寫進設定檔，否則這個測試證明不了任何事"
        );
        let cfg: AppConfig = toml::from_str(&serialized).expect("舊設定檔應該仍然載入得了");
        assert!(cfg.remote_hosts.is_empty());
    }

    #[test]
    fn remote_host_roundtrips_toml() {
        // 旁邊就有 vcs_connection_roundtrips_toml（types.rs:814），形狀照它。
        let mut cfg = AppConfig::default();
        cfg.remote_hosts.push(RemoteHost {
            id: "abc".into(),
            name: "辦公室 NAS".into(),
            host: "192.168.1.50".into(),
            port: 8022,
        });
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: AppConfig = toml::from_str(&s).unwrap();
        assert_eq!(back.remote_hosts.len(), 1);
        assert_eq!(back.remote_hosts[0].name, "辦公室 NAS");
        assert_eq!(back.remote_hosts[0].port, 8022);
    }
```

- [ ] **Step 2: 跑測試確認它紅**

Run: `cd src-tauri && cargo test --workspace remote_host 2>&1 | tail -20`
Expected: 編譯失敗，`cannot find type RemoteHost` / `no field remote_hosts`

- [ ] **Step 3: 加型別與欄位**

在 `src-tauri/src/config/types.rs` 的 `VcsConnection` 定義旁邊加入
（照它的 derive；若 `VcsConnection` 的 derive 行與此不同，以該檔案實際的為準）：

```rust
/// A saved remote terminal host. The pre-shared key lives in the OS keychain
/// under `remote:{id}`, never in this file.
//
// 不加 rename_all：這個檔案裡的設定型別（VcsConnection 等）一律維持 snake_case。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteHost {
    pub id: String,
    /// 使用者取的別名，顯示在清單上。
    pub name: String,
    pub host: String,
    pub port: u16,
}
```

在 `AppConfig` 裡、`vcs_connections` 那幾行旁邊加入：

```rust
    /// Saved remote terminal hosts (keys stored separately in Keychain).
    #[serde(default)]
    pub remote_hosts: Vec<RemoteHost>,
```

`AppConfig` 有兩處手寫的預設值建構（`types.rs` 約 306 與 709 行，都有
`vcs_connections: vec![]`）。**兩處都要加** `remote_hosts: vec![]`，漏一處會編譯失敗。

- [ ] **Step 4: 跑測試確認它綠**

Run: `cd src-tauri && cargo test --workspace remote_host 2>&1 | tail -6`
Expected: `test result: ok`，三條都過

- [ ] **Step 5: 加 ConfigStore 的三個存取方法**

在 `src-tauri/src/config/mod.rs` 的 `remove_vcs_connection` 之後加入：

```rust
    /// Add a new remote terminal host.
    pub fn add_remote_host(&self, host: RemoteHost) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.remote_hosts.push(host);
        })
    }

    /// Update an existing remote host by id. Silently no-ops if not found.
    pub fn update_remote_host(&self, host: RemoteHost) -> anyhow::Result<()> {
        self.update(|cfg| {
            if let Some(existing) = cfg.remote_hosts.iter_mut().find(|h| h.id == host.id) {
                *existing = host;
            }
        })
    }

    /// Remove a remote host by id.
    pub fn remove_remote_host(&self, id: &str) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.remote_hosts.retain(|h| h.id != id);
        })
    }
```

檔案頂端的 `use` 要把 `RemoteHost` 一起帶進來（跟 `VcsConnection` 同一行或同一組）。

- [ ] **Step 6: 編譯確認**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -5`
Expected: `Finished`，沒有錯誤

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/config/types.rs src-tauri/src/config/mod.rs
git commit -m "feat(config): 新增 remote_hosts 設定與存取方法"
```

---

## Task 2: `resolve_connect_key` 純函式（本計畫最關鍵的一條規則）

這支函式決定一次連線要用哪個金鑰。抽出來是為了**能在沒有 keychain 的環境測到**
——它是整個功能唯一會靜默把「金鑰遺失」變成「短碼不符」的地方。

**Files:**
- Create: `src-tauri/src/commands/remote_hosts.rs`
- Modify: `src-tauri/src/commands/mod.rs`

- [ ] **Step 1: 先寫會紅的測試**

建立 `src-tauri/src/commands/remote_hosts.rs`，內容先只放測試與型別宣告：

```rust
//! 遠端終端機地址簿。
//!
//! 紀錄存設定檔、金鑰存 OS keychain（key 為 `remote:{id}`），形狀照
//! `commands/vcs.rs`。**金鑰不跨 IPC**：前端永遠拿不到已存的金鑰，
//! 連線時只送 `saved_host_id`，由 `share_viewer_connect` 自己去 keychain 取。

/// 前端用來辨識「這台的金鑰不在這台電腦上」的錯誤字串。
///
/// 走字串是因為整個 command 層都是 `Result<_, String>`；前端有一份同名常數，
/// 兩邊必須一致。
pub const ERR_SAVED_KEY_MISSING: &str = "remote_host_key_missing";

/// 這一筆地址簿條目的金鑰在 keychain 裡的 key。
pub fn remote_host_secret_key(id: &str) -> String {
    format!("remote:{id}")
}

/// 決定這次連線要用哪個金鑰。
///
/// **`saved_host_id` 有值卻查不到金鑰時一定要回 `Err`，絕對不能回 `Ok(None)`。**
/// 回 `Ok(None)` 的話後端會把這次連線當成短碼模式，握手失敗的訊息會指向短碼
/// 不符——使用者看到的原因跟真正的原因毫無關係。`ConnectDialog` 現有註解記錄
/// 的 `Some("")` 陷阱就是同一類問題。
///
/// `lookup` 由呼叫端注入，測試才能在沒有 keychain 的環境跑。
pub fn resolve_connect_key(
    saved_host_id: Option<&str>,
    typed_key: Option<String>,
    lookup: impl FnOnce(&str) -> Option<String>,
) -> Result<Option<String>, String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_secret_key_is_namespaced() {
        // 不加前綴的話會跟 provider id 撞在同一個 keychain 命名空間裡。
        assert_eq!(remote_host_secret_key("abc"), "remote:abc");
    }

    #[test]
    fn a_saved_host_uses_the_key_from_the_keychain() {
        let got = resolve_connect_key(Some("abc"), None, |k| {
            assert_eq!(k, "remote:abc");
            Some("deadbeef".into())
        })
        .unwrap();
        assert_eq!(got, Some("deadbeef".into()));
    }

    #[test]
    fn a_saved_host_with_no_key_in_the_keychain_is_an_error() {
        // **這是這支函式存在的理由。** 回 Ok(None) 的話會被當成短碼模式，
        // 錯誤訊息指向短碼不符，跟真正的原因無關。
        let got = resolve_connect_key(Some("abc"), None, |_| None);
        assert_eq!(got, Err(ERR_SAVED_KEY_MISSING.to_string()));
    }

    #[test]
    fn a_saved_host_ignores_whatever_the_frontend_sent() {
        // 前端在這條路上不該填 key；就算填了也不採用，避免出現兩個來源。
        let got = resolve_connect_key(Some("abc"), Some("from-frontend".into()), |_| {
            Some("from-keychain".into())
        })
        .unwrap();
        assert_eq!(got, Some("from-keychain".into()));
    }

    #[test]
    fn a_manual_connection_uses_the_typed_key() {
        let got = resolve_connect_key(None, Some("typed".into()), |_| {
            panic!("沒有 saved_host_id 時不該去查 keychain");
        })
        .unwrap();
        assert_eq!(got, Some("typed".into()));
    }

    #[test]
    fn a_short_code_connection_has_no_key_at_all() {
        let got = resolve_connect_key(None, None, |_| {
            panic!("沒有 saved_host_id 時不該去查 keychain");
        })
        .unwrap();
        assert_eq!(got, None);
    }
}
```

在 `src-tauri/src/commands/mod.rs` 加入下面這行。位置照字母序——實際的槽位在
`pub mod python_env;` 與 `pub mod reports;` 之間：

```rust
pub mod remote_hosts;
```

- [ ] **Step 2: 跑測試確認它紅**

Run: `cd src-tauri && cargo test --workspace commands::remote_hosts 2>&1 | tail -20`

> **過濾字串要用測試的路徑，不是函式名。** `cargo test <filter>` 比對的是
> 「模組路徑::測試名」，而沒有任何一條測試的名字含有 `resolve_connect_key`。
> 用函式名當過濾字串會匹配到 **0 條測試，而 cargo 對 0 條測試回報 `ok`**——
> 看起來全綠，其實什麼都沒跑。用過濾字串之後一定要看 `running N tests` 的 N。

Expected: 六條測試中，凡是呼叫 `resolve_connect_key` 的都因為 `todo!()` panic 而 FAIL
（`the_secret_key_is_namespaced` 會過——它不碰那支函式）

- [ ] **Step 3: 寫實作**

把 `resolve_connect_key` 的 `todo!()` 換成：

```rust
    match saved_host_id {
        Some(id) => match lookup(&remote_host_secret_key(id)) {
            Some(key) => Ok(Some(key)),
            None => Err(ERR_SAVED_KEY_MISSING.to_string()),
        },
        None => Ok(typed_key),
    }
```

- [ ] **Step 4: 跑測試確認它綠**

Run: `cd src-tauri && cargo test --workspace commands::remote_hosts 2>&1 | tail -6`
Expected: `running 6 tests` 然後 `test result: ok. 6 passed`

- [ ] **Step 5: 證明第三條測試真的會分勝負**

把實作的 `None => Err(...)` 暫時改成 `None => Ok(None)`，重跑：

Run: `cd src-tauri && cargo test --workspace a_saved_host_with_no_key 2>&1 | tail -6`
Expected: **FAILED**。確認之後把 `Err(...)` 改回去，再跑一次確認回到 ok。

（這一步不能跳過。這條規則的失敗模式是靜默的，測試如果不會紅，它就等於不存在。）

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/remote_hosts.rs src-tauri/src/commands/mod.rs
git commit -m "feat(remote): 連線金鑰解析抽成可測的純函式"
```

---

## Task 3: 四個 Tauri 指令

**Files:**
- Modify: `src-tauri/src/commands/remote_hosts.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 先寫會紅的測試**

在 `remote_hosts.rs` 的 `mod tests` 裡加入（這些只碰設定檔，不碰 keychain，
所以**不需要** `#[ignore]`）：

```rust
    use crate::config::ConfigStore;
    use tempfile::tempdir;

    #[test]
    fn adding_a_host_puts_it_in_the_config() {
        let dir = tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let id = insert_host(
            &config,
            None,
            "辦公室".into(),
            "192.168.1.50".into(),
            8022,
        )
        .unwrap();
        let hosts = config.get().remote_hosts;
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].id, id);
        assert_eq!(hosts[0].name, "辦公室");
        assert_eq!(hosts[0].port, 8022);
    }

    #[test]
    fn updating_a_host_keeps_its_id_and_does_not_add_a_second_row() {
        let dir = tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let id = insert_host(&config, None, "舊名".into(), "1.2.3.4".into(), 8022).unwrap();
        insert_host(
            &config,
            Some(id.clone()),
            "新名".into(),
            "1.2.3.4".into(),
            9000,
        )
        .unwrap();
        let hosts = config.get().remote_hosts;
        assert_eq!(hosts.len(), 1, "更新不該再新增一列");
        assert_eq!(hosts[0].id, id, "更新不該換掉 id——keychain 的金鑰是綁 id 的");
        assert_eq!(hosts[0].name, "新名");
        assert_eq!(hosts[0].port, 9000);
    }

    #[test]
    fn removing_a_host_takes_it_out_of_the_config() {
        let dir = tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let id = insert_host(&config, None, "要刪的".into(), "1.2.3.4".into(), 8022).unwrap();
        let keep = insert_host(&config, None, "留著".into(), "5.6.7.8".into(), 8022).unwrap();
        config.remove_remote_host(&id).unwrap();
        let hosts = config.get().remote_hosts;
        assert_eq!(hosts.len(), 1, "只該刪掉指定的那一列");
        assert_eq!(hosts[0].id, keep);
    }
```

- [ ] **Step 2: 跑測試確認它紅**

Run: `cd src-tauri && cargo test --workspace adding_a_host 2>&1 | tail -12`
Expected: 編譯失敗，`cannot find function insert_host`

- [ ] **Step 3: 寫實作**

在 `remote_hosts.rs` 的 `resolve_connect_key` 之後（`mod tests` 之前）加入：

```rust
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::config::{ConfigStore, RemoteHost};
use crate::secret::SecretStore;

/// 前端送進來的整筆資料。`id` 為 `None` 代表新增。
#[derive(Debug, Deserialize)]
pub struct RemoteHostInput {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// 預共享金鑰。**只往這個方向走**——列出時永遠不回傳它。
    pub secret: Option<String>,
}

/// 回給前端的資料，**不含金鑰**。
#[derive(Debug, Serialize)]
pub struct RemoteHostInfo {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// 這台電腦的 keychain 裡有沒有這一筆的金鑰。設定檔被同步到另一台電腦時
    /// 會是 false——UI 靠它先提醒，而不是等連線失敗。
    pub has_key: bool,
}

/// 寫入設定檔（不碰 keychain），回傳這一筆的 id。
///
/// 跟兩個 command 共用，也讓設定檔那半邊在沒有 keychain 的環境測得到。
fn insert_host(
    config: &ConfigStore,
    id: Option<String>,
    name: String,
    host: String,
    port: u16,
) -> Result<String, String> {
    match id {
        Some(id) => {
            let record = RemoteHost { id: id.clone(), name, host, port };
            config.update_remote_host(record).map_err(|e| e.to_string())?;
            Ok(id)
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            let record = RemoteHost { id: id.clone(), name, host, port };
            config.add_remote_host(record).map_err(|e| e.to_string())?;
            Ok(id)
        }
    }
}

#[tauri::command]
pub async fn remote_hosts_list(
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<RemoteHostInfo>, String> {
    Ok(config
        .get()
        .remote_hosts
        .into_iter()
        .map(|h| RemoteHostInfo {
            has_key: secrets.has(&remote_host_secret_key(&h.id)),
            id: h.id,
            name: h.name,
            host: h.host,
            port: h.port,
        })
        .collect())
}

#[tauri::command]
pub async fn remote_hosts_add(
    input: RemoteHostInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let id = insert_host(&config, None, input.name, input.host, input.port)?;
    if let Some(secret) = &input.secret {
        if !secret.is_empty() {
            secrets
                .set(&remote_host_secret_key(&id), secret)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(id)
}

#[tauri::command]
pub async fn remote_hosts_update(
    input: RemoteHostInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let id = input.id.clone().ok_or("missing id")?;
    insert_host(&config, Some(id.clone()), input.name, input.host, input.port)?;
    // 空字串代表「這次沒有要改金鑰」，不是「把金鑰清空」。照抄 vcs.rs 的語意：
    // 編輯對話框不會把既有金鑰回填（前端根本拿不到），所以送空值必須是無操作，
    // 否則使用者只改個別名就會把金鑰弄丟。
    if let Some(secret) = &input.secret {
        if !secret.is_empty() {
            secrets
                .set(&remote_host_secret_key(&id), secret)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn remote_hosts_remove(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    config.remove_remote_host(&id).map_err(|e| e.to_string())?;
    // 設定檔刪了金鑰卻留著＝keychain 裡累積永遠不會再被用到的條目。
    // 刪失敗不該讓整個操作失敗（條目可能本來就不存在）。
    let _ = secrets.delete(&remote_host_secret_key(&id));
    Ok(())
}
```

- [ ] **Step 4: 跑測試確認它綠**

Run: `cd src-tauri && cargo test --workspace remote_hosts 2>&1 | tail -6`
Expected: `test result: ok`，Task 2 的六條加這裡的三條全過

- [ ] **Step 5: 加兩條碰 keychain 的測試（標 `#[ignore]`）**

spec 要求驗「更新不帶 secret 不會清掉金鑰」與「刪除會一併刪 keychain」。
這兩件事一定要碰真實 keychain，所以照 `secret/mod.rs:199` 的既有慣例標
`#[ignore]`——CI 不跑，開發者手動跑。

在 `mod tests` 裡加入：

```rust
    /// 這兩條碰真實 OS keychain，照 secret/mod.rs 的慣例標 #[ignore]。
    /// 手動跑：`cargo test --workspace remote_hosts_keychain -- --ignored`
    #[test]
    #[ignore]
    fn remote_hosts_keychain_update_without_a_secret_keeps_the_existing_key() {
        let secrets = SecretStore::new();
        let id = format!("test-{}", uuid::Uuid::new_v4());
        let key = remote_host_secret_key(&id);
        secrets.set(&key, "original").unwrap();

        // 模擬 remote_hosts_update 收到 secret: None（使用者只改了別名）。
        let incoming: Option<String> = None;
        if let Some(s) = &incoming {
            if !s.is_empty() {
                secrets.set(&key, s).unwrap();
            }
        }

        assert_eq!(secrets.get(&key).unwrap(), Some("original".into()));
        secrets.delete(&key).unwrap();
    }

    #[test]
    #[ignore]
    fn remote_hosts_keychain_remove_deletes_the_key() {
        let dir = tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = SecretStore::new();
        let id = insert_host(&config, None, "要刪的".into(), "1.2.3.4".into(), 8022).unwrap();
        let key = remote_host_secret_key(&id);
        secrets.set(&key, "deadbeef").unwrap();
        assert!(secrets.has(&key), "前置條件：金鑰要先真的存進去");

        config.remove_remote_host(&id).unwrap();
        let _ = secrets.delete(&key);

        assert!(!secrets.has(&key), "刪掉條目之後 keychain 不該還留著金鑰");
    }
```

- [ ] **Step 6: 手動跑那兩條，確認它們真的會過**

Run: `cd src-tauri && cargo test --workspace remote_hosts_keychain -- --ignored 2>&1 | tail -6`
Expected: `test result: ok. 2 passed`

macOS 可能會跳出 keychain 存取授權視窗，按「允許」。若這台機器沒有可用的
keychain（例如沒有跑 SecretService 的 Linux），這兩條會失敗——那是環境問題，
不是程式問題，記下來即可，**不要**為了讓它綠而改動實作。

- [ ] **Step 7: 註冊指令**

`src-tauri/src/lib.rs` 有兩處要改（檔案上方的 `use` 清單、以及
`invoke_handler` 的 `generate_handler!` 清單）。

在 `use` 區塊裡（約 106-126 行那一帶，維持字母序）加入：

```rust
        remote_hosts_add, remote_hosts_list, remote_hosts_remove, remote_hosts_update,
```

在 `generate_handler!` 的清單裡（`share_viewer_connect` 那一行附近）加入：

```rust
            remote_hosts_list,
            remote_hosts_add,
            remote_hosts_update,
            remote_hosts_remove,
```

`use` 的路徑前綴照該檔案既有的寫法（跟 `vcs_add_connection` 同一組）。

- [ ] **Step 8: 編譯確認**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -5`
Expected: `Finished`

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/commands/remote_hosts.rs src-tauri/src/lib.rs
git commit -m "feat(remote): 地址簿的四個指令"
```

---

## Task 4: `share_viewer_connect` 接上 `saved_host_id`

**Files:**
- Modify: `src-tauri/src/commands/share_viewer.rs:13-26`

- [ ] **Step 1: 改簽章與內容**

把 `src-tauri/src/commands/share_viewer.rs` 的 `share_viewer_connect` 換成：

```rust
#[tauri::command]
pub async fn share_viewer_connect(
    host: String,
    port: u16,
    code: String,
    display_name: String,
    key: Option<String>,
    /// 地址簿的條目 id。帶了它就由後端自己去 keychain 取金鑰，`key` 不採用——
    /// 已存的金鑰因此從頭到尾不跨 IPC。
    saved_host_id: Option<String>,
    viewers: State<'_, Arc<ViewerManager>>,
    secrets: State<'_, Arc<SecretStore>>,
    app: AppHandle,
) -> Result<Connected, String> {
    // **不要寫成 `secrets.get(k).ok().flatten()`。** SecretStore::get 回的是
    // Result<Option<String>>：Ok(None) 是「這台沒有這把金鑰」，Err 是「keychain
    // 讀不到」。壓成同一個 None 的話，keychain 鎖住會顯示成「金鑰不在這台電腦
    // 上，請重新輸入」，使用者就會去重貼一把其實好好的金鑰。
    let key = resolve_connect_key(saved_host_id.as_deref(), key, |k| {
        // 用 `{e:#}` 而不是 `.to_string()`：SecretStore 內部用 anyhow 的
        // with_context 包了一層，`.to_string()` 只會拿到最外層那句
        // 「opening keychain entry for ...」，真正的原因被吃掉。
        // （write_entry 已經為寫入路徑修過同一個問題，讀取路徑沒有。）
        secrets.get(k).map_err(|e| format!("{e:#}"))
    })?;
    viewers
        .connect(app, host, port, code, display_name, key)
        .await
        .map_err(|e| format!("{e}"))
}
```

檔案頂端補上：

```rust
use crate::commands::remote_hosts::resolve_connect_key;
use crate::secret::SecretStore;
```

（`std::sync::Arc` 與 `tauri::State` 這個檔案已經有了。）

- [ ] **Step 2: 編譯確認**

Run: `cd src-tauri && cargo check --workspace 2>&1 | tail -5`
Expected: `Finished`

- [ ] **Step 3: 確認既有行為沒被改壞**

Run: `cd src-tauri && cargo test --workspace 2>&1 | tail -8`
Expected: 全部 `ok`。既有的分享／觀看端測試不該有任何一條紅——
`saved_host_id` 為 `None` 時 `resolve_connect_key` 直接把 `key` 原樣回傳。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/share_viewer.rs
git commit -m "feat(remote): share_viewer_connect 支援從地址簿取金鑰"
```

---

## Task 5: 前端 IPC 包裝

**Files:**
- Create: `src/ipc/remoteHosts.ts`
- Modify: `src/ipc/shareViewer.ts`

- [ ] **Step 1: 建立 `src/ipc/remoteHosts.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";

/**
 * 「這台的金鑰不在這台電腦上」。跟 Rust 的
 * `commands::remote_hosts::ERR_SAVED_KEY_MISSING` 必須一字不差。
 */
export const ERR_SAVED_KEY_MISSING = "remote_host_key_missing";

/** 地址簿的一筆，**永遠不含金鑰**。 */
export interface RemoteHostInfo {
  id: string;
  name: string;
  host: string;
  port: number;
  /** 這台電腦的 keychain 裡有沒有它的金鑰。設定檔同步到別台時會是 false。
   *  欄位名維持 snake_case——這個 repo 的 IPC 型別都是（見 src/ipc/vcs.ts 的 has_secret）。 */
  has_key: boolean;
}

export interface RemoteHostInput {
  /** 省略代表新增。 */
  id?: string;
  name: string;
  host: string;
  port: number;
  /** 預共享金鑰。**空字串或省略代表「這次不改金鑰」，不是「清空金鑰」**。 */
  secret?: string;
}

export function remoteHostsList(): Promise<RemoteHostInfo[]> {
  return invoke<RemoteHostInfo[]>("remote_hosts_list");
}

export function remoteHostsAdd(input: RemoteHostInput): Promise<string> {
  return invoke<string>("remote_hosts_add", { input });
}

export function remoteHostsUpdate(input: RemoteHostInput): Promise<void> {
  return invoke<void>("remote_hosts_update", { input });
}

export function remoteHostsRemove(id: string): Promise<void> {
  return invoke<void>("remote_hosts_remove", { id });
}
```

- [ ] **Step 2: `shareViewer.ts` 加上 `savedHostId`**

把 `ShareViewerConnectArgs` 改成：

```ts
export interface ShareViewerConnectArgs {
  host: string;
  port: number;
  code: string;
  displayName: string;
  /** CLI host 的預共享金鑰（hex）。短碼模式與地址簿模式都留空。 */
  key?: string;
  /**
   * 地址簿的條目 id。帶了它就由**後端**去 keychain 取金鑰——已存的金鑰
   * 不跨 IPC，前端從頭到尾拿不到它。帶了這個就不要再帶 `key`。
   */
  savedHostId?: string;
}
```

把 `shareViewerConnect` 的 `invoke` 參數加一行：

```ts
    key: args.key,
    savedHostId: args.savedHostId,
```

- [ ] **Step 3: 型別檢查**

Run: `npx tsc -b 2>&1 | tail -5`
Expected: 沒有輸出（通過）

> 注意：不要用 `tsc --noEmit`。根目錄的 `tsconfig.toml` 是 solution file
> （`"files": []`），那樣跑什麼都不會檢查而且永遠 exit 0（見 CLAUDE.md）。

- [ ] **Step 4: Commit**

```bash
git add src/ipc/remoteHosts.ts src/ipc/shareViewer.ts
git commit -m "feat(remote): 地址簿的前端 IPC 包裝"
```

---

## Task 6: i18n 字串

先做 i18n，後面兩個 Task 的元件才有字串可用。

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 加 zh-TW 字串**

在 `src/lib/i18n.ts` 的 zh-TW 區塊、`connect_failed`（約 1021 行）旁邊加入：

```ts
    connect_saved_title: "已儲存的主機",
    connect_saved_connect: "連線",
    connect_saved_edit: "編輯",
    connect_saved_delete: "刪除",
    connect_saved_delete_confirm: "確定要從地址簿刪除「{name}」嗎？金鑰也會一併刪除。",
    connect_saved_no_key: "這台的金鑰不在這台電腦上，請重新輸入。",
    connect_save_prompt: "已連上。要把這台存進地址簿嗎？",
    connect_save_name_label: "別名",
    connect_save_confirm: "儲存",
    connect_save_skip: "不用",
```

- [ ] **Step 2: 加 en 字串**

在 en 區塊、`connect_failed`（約 2593 行）旁邊加入：

```ts
    connect_saved_title: "Saved hosts",
    connect_saved_connect: "Connect",
    connect_saved_edit: "Edit",
    connect_saved_delete: "Delete",
    connect_saved_delete_confirm:
      "Remove \"{name}\" from the address book? Its key will be deleted too.",
    connect_saved_no_key: "This host's key is not on this computer. Enter it again.",
    connect_save_prompt: "Connected. Save this host to the address book?",
    connect_save_name_label: "Name",
    connect_save_confirm: "Save",
    connect_save_skip: "Not now",
```

- [ ] **Step 3: 型別檢查（會抓到只加一邊的情況）**

Run: `npx tsc -b 2>&1 | tail -5`
Expected: 沒有輸出。只加一個語系的話這裡會紅——兩份的鍵必須一致。

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(i18n): 地址簿字串（en / zh-TW）"
```

---

## Task 7: `RemoteHostList` 元件

**Files:**
- Create: `src/components/ConnectDialog/RemoteHostList.tsx`
- Test: `src/components/ConnectDialog/RemoteHostList.test.tsx`

- [ ] **Step 1: 先寫會紅的測試**

建立 `src/components/ConnectDialog/RemoteHostList.test.tsx`：

```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RemoteHostList } from "./RemoteHostList";
import { LocaleProvider } from "../../contexts/LocaleContext";

const hosts = [
  { id: "a", name: "辦公室", host: "192.168.1.50", port: 8022, has_key: true },
  { id: "b", name: "雲端", host: "10.0.0.9", port: 9000, has_key: false },
];

function renderList(props: Partial<Parameters<typeof RemoteHostList>[0]> = {}) {
  return render(
    <LocaleProvider>
      <RemoteHostList
        hosts={hosts}
        onConnect={props.onConnect ?? vi.fn()}
        onEdit={props.onEdit ?? vi.fn()}
        onDelete={props.onDelete ?? vi.fn()}
      />
    </LocaleProvider>,
  );
}

describe("RemoteHostList", () => {
  it("每一筆都看得到別名與位址", () => {
    renderList();
    expect(screen.getByText("辦公室")).toBeInTheDocument();
    expect(screen.getByText("192.168.1.50:8022")).toBeInTheDocument();
    expect(screen.getByText("雲端")).toBeInTheDocument();
    expect(screen.getByText("10.0.0.9:9000")).toBeInTheDocument();
  });

  it("點一列就帶著那一筆呼叫 onConnect", async () => {
    const onConnect = vi.fn();
    renderList({ onConnect });
    await userEvent.click(screen.getByText("辦公室"));
    expect(onConnect).toHaveBeenCalledTimes(1);
    expect(onConnect.mock.calls[0][0].id).toBe("a");
  });

  it("金鑰不在這台電腦上的那一筆會標示出來", () => {
    // 不標的話使用者只會看到連線失敗，不知道要重貼金鑰。
    renderList();
    const rows = screen.getAllByRole("listitem");
    expect(rows[1].textContent).toContain("金鑰");
    expect(rows[0].textContent).not.toContain("金鑰");
  });

  it("沒有任何一筆時整塊都不渲染", () => {
    render(
      <LocaleProvider>
        <RemoteHostList hosts={[]} onConnect={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />
      </LocaleProvider>,
    );
    expect(screen.queryByRole("list")).toBeNull();
  });
});
```

- [ ] **Step 2: 跑測試確認它紅**

Run: `npm run test -- RemoteHostList 2>&1 | tail -12`
Expected: FAIL — 找不到模組 `./RemoteHostList`

- [ ] **Step 3: 寫元件**

建立 `src/components/ConnectDialog/RemoteHostList.tsx`：

```tsx
import type { RemoteHostInfo } from "../../ipc/remoteHosts";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  hosts: RemoteHostInfo[];
  onConnect: (host: RemoteHostInfo) => void;
  onEdit: (host: RemoteHostInfo) => void;
  onDelete: (host: RemoteHostInfo) => void;
}

/**
 * 地址簿清單。
 *
 * 從 `ConnectDialog` 抽出來：那個元件原本已經同時負責模式切換、mDNS 搜尋與
 * 送出，再加上清單管理會變成四件事。
 *
 * 沒有任何條目時整塊不渲染——空清單的標題對沒用過這個功能的人只是雜訊。
 */
export function RemoteHostList({ hosts, onConnect, onEdit, onDelete }: Props) {
  const { t } = useLocale();
  if (hosts.length === 0) return null;
  return (
    <div className="aiterm-connect__saved">
      <div className="aiterm-connect__label">{t.connect_saved_title}</div>
      <ul className="aiterm-connect__saved-list" role="list">
        {hosts.map((h) => (
          <li key={h.id} className="aiterm-connect__saved-row">
            <button
              type="button"
              className="aiterm-connect__saved-main"
              onClick={() => onConnect(h)}
              title={t.connect_saved_connect}
            >
              <span className="aiterm-connect__saved-name">{h.name}</span>
              <span className="aiterm-connect__saved-addr">{`${h.host}:${h.port}`}</span>
              {/* 金鑰不在本機時先說清楚，不要等連線失敗才讓使用者猜。 */}
              {!h.has_key && (
                <span className="aiterm-connect__saved-warn">{t.connect_saved_no_key}</span>
              )}
            </button>
            <button
              type="button"
              className="aiterm-connect__saved-action"
              onClick={() => onEdit(h)}
            >
              {t.connect_saved_edit}
            </button>
            <button
              type="button"
              className="aiterm-connect__saved-action"
              onClick={() => onDelete(h)}
            >
              {t.connect_saved_delete}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
```

在 `src/components/ConnectDialog/index.css` 最底下加入：

```css
.aiterm-connect__saved { margin-bottom: 12px; }
.aiterm-connect__saved-list { list-style: none; margin: 0; padding: 0; }
.aiterm-connect__saved-row { display: flex; align-items: center; gap: 6px; }
.aiterm-connect__saved-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: 6px 8px;
  background: none;
  border: none;
  cursor: pointer;
  text-align: left;
  color: inherit;
}
.aiterm-connect__saved-main:hover { background: rgba(127, 127, 127, 0.15); }
.aiterm-connect__saved-name { font-weight: 600; }
.aiterm-connect__saved-addr { opacity: 0.7; font-size: 0.85em; }
.aiterm-connect__saved-warn { color: #d08770; font-size: 0.8em; }
.aiterm-connect__saved-action {
  background: none;
  border: none;
  cursor: pointer;
  opacity: 0.7;
  color: inherit;
  font-size: 0.85em;
}
.aiterm-connect__saved-action:hover { opacity: 1; }
.aiterm-connect__confirm,
.aiterm-connect__save {
  margin: 8px 0;
  padding: 8px;
  border: 1px solid rgba(127, 127, 127, 0.3);
  border-radius: 4px;
}
```

- [ ] **Step 4: 跑測試確認它綠**

Run: `npm run test -- RemoteHostList 2>&1 | tail -8`
Expected: 4 passed

- [ ] **Step 5: Commit**

```bash
git add src/components/ConnectDialog/RemoteHostList.tsx \
        src/components/ConnectDialog/RemoteHostList.test.tsx \
        src/components/ConnectDialog/index.css
git commit -m "feat(remote): 地址簿清單元件"
```

---

## Task 8: 接進 `ConnectDialog`

**Files:**
- Modify: `src/components/ConnectDialog/index.tsx`
- Test: `src/components/ConnectDialog/index.test.tsx`

- [ ] **Step 1: 先寫會紅的測試**

在 `src/components/ConnectDialog/index.test.tsx` 現有的測試之後加入。
先確認該檔案頂端已經 mock 了 `../../ipc/shareViewer` 與 `../../ipc/share`；
若沒有，照它既有的 mock 寫法補上，並加上 `../../ipc/remoteHosts` 的 mock：

```tsx
vi.mock("../../ipc/remoteHosts", () => ({
  ERR_SAVED_KEY_MISSING: "remote_host_key_missing",
  remoteHostsList: vi.fn(async () => []),
  remoteHostsAdd: vi.fn(async () => "new-id"),
  remoteHostsUpdate: vi.fn(async () => undefined),
  remoteHostsRemove: vi.fn(async () => undefined),
}));
```

測試本體：

```tsx
describe("ConnectDialog 地址簿", () => {
  beforeEach(() => {
    vi.mocked(remoteHostsList).mockResolvedValue([
      { id: "a", name: "辦公室", host: "192.168.1.50", port: 8022, has_key: true },
    ]);
    vi.mocked(shareViewerConnect).mockResolvedValue({ connId: "c1", sas: "1234" });
  });

  it("點清單的一筆時只送 savedHostId，不送 key", async () => {
    // 金鑰不跨 IPC 是整個設計的核心。前端若自己帶 key，代表它某處拿得到
    // 已存的金鑰——那就是這個設計要避免的事。
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByText("辦公室"));
    expect(shareViewerConnect).toHaveBeenCalledTimes(1);
    const args = vi.mocked(shareViewerConnect).mock.calls[0][0];
    expect(args.savedHostId).toBe("a");
    expect(args.key).toBeUndefined();
  });

  it("從地址簿連上之後不再問要不要儲存", async () => {
    const onConnected = vi.fn();
    render(<ConnectDialog onConnected={onConnected} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByText("辦公室"));
    expect(screen.queryByText(/存進地址簿/)).toBeNull();
    expect(onConnected).toHaveBeenCalledWith("c1", "1234", "192.168.1.50:8022");
  });

  it("手動輸入金鑰連上之後會問要不要儲存，按儲存才寫入", async () => {
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(screen.getByText(/▸/));
    await userEvent.type(screen.getByLabelText(/位址|Address/), "10.0.0.9:9000");
    await userEvent.type(screen.getByLabelText(/金鑰|Key/), "deadbeef");
    await userEvent.click(screen.getByRole("button", { name: /連線|Connect$/ }));
    await screen.findByText(/存進地址簿/);
    await userEvent.type(screen.getByLabelText(/別名|^Name$/), "雲端");
    await userEvent.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    expect(remoteHostsAdd).toHaveBeenCalledWith({
      name: "雲端",
      host: "10.0.0.9",
      port: 9000,
      secret: "deadbeef",
    });
  });

  it("按「不用」仍然會開分頁", async () => {
    // 不存 ≠ 丟掉這條已經建立好的連線。
    const onConnected = vi.fn();
    render(<ConnectDialog onConnected={onConnected} onCancel={vi.fn()} />);
    await userEvent.click(screen.getByText(/▸/));
    await userEvent.type(screen.getByLabelText(/位址|Address/), "10.0.0.9:9000");
    await userEvent.type(screen.getByLabelText(/金鑰|Key/), "deadbeef");
    await userEvent.click(screen.getByRole("button", { name: /連線|Connect$/ }));
    await userEvent.click(await screen.findByRole("button", { name: /^不用$|Not now/ }));
    expect(remoteHostsAdd).not.toHaveBeenCalled();
    expect(onConnected).toHaveBeenCalledWith("c1", "1234", "10.0.0.9:9000");
  });

  it("刪除要先確認，按確認才真的刪", async () => {
    // 刪除會連 keychain 的金鑰一起刪掉，一按就生效太危險。
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.click(await screen.findByRole("button", { name: /^刪除$|^Delete$/ }));
    expect(remoteHostsRemove).not.toHaveBeenCalled();
    await screen.findByText(/辦公室/);
    const buttons = screen.getAllByRole("button", { name: /^刪除$|^Delete$/ });
    await userEvent.click(buttons[buttons.length - 1]);
    expect(remoteHostsRemove).toHaveBeenCalledWith("a");
  });

  it("短碼模式連上之後不問儲存", async () => {
    // 短碼每次都不一樣，存起來沒有意義。
    vi.mocked(shareDiscover).mockResolvedValue({
      kind: "found",
      host: "1.2.3.4",
      port: 8022,
    });
    render(<ConnectDialog onConnected={vi.fn()} onCancel={vi.fn()} />);
    await userEvent.type(screen.getByLabelText(/短碼|Code/), "123456");
    await userEvent.click(screen.getByRole("button", { name: /連線|Connect$/ }));
    await waitFor(() => expect(shareViewerConnect).toHaveBeenCalled());
    expect(screen.queryByText(/存進地址簿/)).toBeNull();
  });
});
```

> 送出鈕用 `name: /連線|Connect$/` 而不是純字串：清單每一列也有「連線」
> 的 title，純字串會撈到多個元素。若仍有歧義，改用 `getAllByRole` 取最後一個
> （送出鈕在對話框最底下），並在測試裡註明原因。

- [ ] **Step 2: 跑測試確認它紅**

Run: `npm run test -- ConnectDialog 2>&1 | tail -15`
Expected: 新的 describe 全部 FAIL（找不到「辦公室」、找不到「存進地址簿」等）

- [ ] **Step 3: 改 `ConnectDialog/index.tsx`**

加入 import：

```tsx
import { useEffect } from "react";
import { RemoteHostList } from "./RemoteHostList";
import {
  ERR_SAVED_KEY_MISSING,
  remoteHostsAdd,
  remoteHostsList,
  remoteHostsRemove,
  remoteHostsUpdate,
  type RemoteHostInfo,
} from "../../ipc/remoteHosts";
```

在既有的 `useState` 之後加入狀態：

```tsx
  const [hosts, setHosts] = useState<RemoteHostInfo[]>([]);
  /** 連上之後、還沒決定要不要存的那筆。null 代表沒有待決定的。 */
  const [pendingSave, setPendingSave] = useState<{
    host: string;
    port: number;
    secret: string;
    connId: string;
    sas: string;
    label: string;
  } | null>(null);
  const [saveName, setSaveName] = useState("");

  useEffect(() => {
    void remoteHostsList().then(setHosts).catch(() => setHosts([]));
  }, []);
```

把 `connectTo` 改成接受來源資訊，並在手動金鑰模式時改為進入待存狀態：

```tsx
  async function connectTo(
    host: string,
    port: number,
    addressLabel: string,
    opts: { savedHostId?: string; typedKey?: string } = {},
  ) {
    try {
      const { connId, sas } = await shareViewerConnect({
        host,
        port,
        code,
        displayName: name || "AITerm",
        // 空字串要送 undefined，不能送 ""。後端把 Some("") 當成「有金鑰但是空的」，
        // 握手會失敗，而錯誤訊息會指向金鑰不符——對一個根本沒填金鑰的使用者來說
        // 完全誤導。
        key: opts.savedHostId ? undefined : opts.typedKey,
        savedHostId: opts.savedHostId,
      });
      // 只有「手動輸入的金鑰模式」才問要不要存：短碼沒有固定金鑰可存，
      // 從地址簿來的本來就存過了。
      if (!opts.savedHostId && opts.typedKey) {
        setPendingSave({ host, port, secret: opts.typedKey, connId, sas, label: addressLabel });
        return;
      }
      onConnected(connId, sas, addressLabel);
    } catch (e) {
      const msg = String(e);
      if (msg.includes(ERR_SAVED_KEY_MISSING)) {
        // 金鑰不在這台電腦上：把位址帶進手動欄位，使用者只要重貼金鑰。
        // 絕對不能靜默改用短碼模式重試——那會得到一個指向短碼的錯誤訊息。
        setManualOpen(true);
        setAddress(addressLabel);
        setKey("");
        setError(t.connect_saved_no_key);
        return;
      }
      // 連不上要說原因，不要靜默關閉——使用者才知道下一步該做什麼。
      setError(t.connect_failed.replace("{error}", msg));
    }
  }
```

`submit()` 裡手動那條路徑改成帶上打字的金鑰：

```tsx
      await connectTo(parsed.host, parsed.port, address, {
        typedKey: key.trim() === "" ? undefined : key.trim(),
      });
```

mDNS 那條路徑維持原呼叫（不傳 `opts`）。

加入清單的三個處理函式：

```tsx
  async function refresh() {
    setHosts(await remoteHostsList());
  }

  async function removeHost(h: RemoteHostInfo) {
    await remoteHostsRemove(h.id);
    setConfirmDelete(null);
    await refresh();
  }

> **編輯一筆已經不存在的條目會留下孤兒金鑰。** `ConfigStore::update_remote_host`
> 找不到 id 時是靜默 no-op（`config/mod.rs` 的註解明講），所以
> `remote_hosts_update` 對一個已被刪掉的 id 呼叫會回 `Ok`，設定檔沒變，
> 但金鑰照樣被寫進 keychain，留下永遠不會被用到的條目。這是從 `vcs.rs` 原封
> 不動抄來的既有行為，不是地址簿引入的。前端要避免踩到：**編輯前先
> `remoteHostsList()` 重抓一次**，找不到該 id 就重整清單並提示，不要拿畫面上
> 可能已經過期的那一筆直接送出。

  function editHost(h: RemoteHostInfo) {
    // 編輯＝把這一筆帶進手動欄位，金鑰留空（前端拿不到已存的金鑰）。
    // 使用者只改別名時送空 secret，後端會當成「不改金鑰」。
    setManualOpen(true);
    setAddress(`${h.host}:${h.port}`);
    setKey("");
    setEditingId(h.id);
  }
```

並加上這兩個狀態：

```tsx
  const [editingId, setEditingId] = useState<string | null>(null);
  /** 等待確認刪除的那一筆。刪除會連 keychain 的金鑰一起刪掉，不能一按就生效。 */
  const [confirmDelete, setConfirmDelete] = useState<RemoteHostInfo | null>(null);
```

確認列刻意做成就地渲染，**不用 `window.confirm`**：這個 repo 的工作看板已經
因為原生對話框吃過虧（StrictMode 雙呼叫會開兩個、第一個結果被丟掉導致卡死），
而且在 dialog 上疊原生 modal 本來就難用。

存檔提示的送出：

```tsx
  async function confirmSave() {
    if (!pendingSave) return;
    if (editingId) {
      await remoteHostsUpdate({
        id: editingId,
        name: saveName || pendingSave.label,
        host: pendingSave.host,
        port: pendingSave.port,
        secret: pendingSave.secret,
      });
    } else {
      await remoteHostsAdd({
        name: saveName || pendingSave.label,
        host: pendingSave.host,
        port: pendingSave.port,
        secret: pendingSave.secret,
      });
    }
    finishPending();
  }

  function finishPending() {
    if (!pendingSave) return;
    const p = pendingSave;
    setPendingSave(null);
    setSaveName("");
    setEditingId(null);
    onConnected(p.connId, p.sas, p.label);
  }
```

在 JSX 裡，標題之後、短碼欄位之前插入清單：

```tsx
        <RemoteHostList
          hosts={hosts}
          onConnect={(h) => void connectTo(h.host, h.port, `${h.host}:${h.port}`, { savedHostId: h.id })}
          onEdit={editHost}
          onDelete={setConfirmDelete}
        />

        {confirmDelete && (
          <div className="aiterm-connect__confirm">
            <div>
              {t.connect_saved_delete_confirm.replace("{name}", confirmDelete.name)}
            </div>
            <div className="aiterm-connect__actions">
              <button
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                onClick={() => setConfirmDelete(null)}
              >
                {t.connect_cancel}
              </button>
              <button
                className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                onClick={() => void removeHost(confirmDelete)}
              >
                {t.connect_saved_delete}
              </button>
            </div>
          </div>
        )}
```

在錯誤訊息之後、`aiterm-connect__actions` 之前插入存檔提示：

```tsx
        {pendingSave && (
          <div className="aiterm-connect__save">
            <div>{t.connect_save_prompt}</div>
            <label className="aiterm-connect__label" htmlFor="aiterm-connect-savename">
              {t.connect_save_name_label}
            </label>
            <input
              id="aiterm-connect-savename"
              className="aiterm-connect__text"
              type="text"
              value={saveName}
              onChange={(e) => setSaveName(e.target.value)}
            />
            <div className="aiterm-connect__actions">
              <button
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                onClick={finishPending}
              >
                {t.connect_save_skip}
              </button>
              <button
                className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                onClick={() => void confirmSave()}
              >
                {t.connect_save_confirm}
              </button>
            </div>
          </div>
        )}
```

- [ ] **Step 4: 跑測試確認它綠**

Run: `npm run test -- ConnectDialog 2>&1 | tail -10`
Expected: 既有測試加新的五條全過

- [ ] **Step 5: Commit**

```bash
git add src/components/ConnectDialog/index.tsx src/components/ConnectDialog/index.test.tsx
git commit -m "feat(remote): 連線對話框接上地址簿與存檔提示"
```

---

## Task 9: 完整驗證

**Files:** 無

- [ ] **Step 1: 前端全套**

Run: `npm run test 2>&1 | tail -8`
Expected: 全綠

- [ ] **Step 2: 型別檢查**

Run: `npx tsc -b 2>&1 | tail -5`
Expected: 沒有輸出

- [ ] **Step 3: Lint**

Run: `npm run lint 2>&1 | tail -8`
Expected: 沒有錯誤

- [ ] **Step 4: Rust 全套**

Run: `cd src-tauri && cargo test --workspace 2>&1 | tail -10`
Expected: 全綠。**一定要有 `--workspace`**——沒有的話只會跑 `app`，
`aiterm-core` 與 `aiterm-host` 整批被跳過而且沒有任何徵兆。

- [ ] **Step 5: 真機驗收（照 `run` skill）**

Run: `npm run tauri:dev`

在實際視窗裡走一遍：

1. 開「連線到遠端終端機」——第一次應該看不到清單（沒有條目）
2. 用 `aiterm-host --print-connection` 印出來的位址與金鑰手動連上
3. 確認跳出「要把這台存進地址簿嗎？」，填別名、按儲存
4. 關掉分頁，重開連線對話框——清單應該出現那一筆
5. 點那一筆，確認**完全不用打字**就連上，而且**不再問**要不要儲存
6. 按刪除，確認那一筆消失

第 5 步是整個功能的驗收標準。

- [ ] **Step 6: 確認金鑰真的進了 keychain 而不是設定檔**

Run: `grep -c "$(你剛才用的金鑰前 8 碼)" ~/Library/Application\ Support/*/config.toml 2>/dev/null || echo 0`
Expected: `0`。金鑰若出現在設定檔裡，代表 `RemoteHost` 結構被誤加了欄位——
那等於把一把能拿到 shell 的憑證寫進明文檔案。

（設定檔的實際路徑依平台而異；用 `ConfigStore` 的 `config_path()` 或在 app 裡
印出來確認。）

- [ ] **Step 7: Commit（若真機驗收有修東西）**

```bash
git add -A
git commit -m "fix(remote): 真機驗收修正"
```
