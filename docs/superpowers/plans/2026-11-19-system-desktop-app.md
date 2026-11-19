# 系統連線支援桌面 App 實作計畫

Spec：`docs/superpowers/specs/2026-11-19-system-desktop-app-design.md`
驗證指令：`cd src-tauri && cargo test --workspace --no-fail-fast`（看每一行 `test result:`）、`npx tsc -b`、`npm run test`、`npm run lint`。
每個任務：先寫會紅的測試 → 實作 → 綠燈 → 只 `git add` 明確路徑後 commit。

## 任務 1：資料型別與儲存檢查
- `systems/types.rs`：`SystemKind { Web, Desktop }`（snake_case、預設 Web）；`SystemConnection` 加 `kind`、`app: String`、`launch_app: bool`（預設 true），皆 `#[serde(default)]`；補 `Default` 讓既有測試用 `..Default::default()`。
- 修所有 struct literal（`gate.rs`、`skill.rs`、`state.rs`、`intake/runtime.rs`、`commands/systems.rs` 測試）。
- `commands/systems.rs::validate`：桌面類型只檢查名稱、`app`（非空、無控制字元、長度）、model；網頁照舊。
- 測試：舊 JSON（沒有 kind）反序列化為 Web；桌面 validate 通過／拒絕空 app。

## 任務 2：動作層閘門（純函式）
- 新增 `systems/desktop/mod.rs`、`systems/desktop/gate.rs`：`DesktopAction`（snake 名稱字串）、`decide_action(conn, action, label, declared_write) -> (Decision, escalated: bool)`。
- 規則：見 spec「寫入閘門」。`Rules` 模式比對 `method`＝動作、`url_pattern`＝標題 glob（用 `gate::glob_match`）。
- 測試：四種模式×讀／寫／危險字樣／Enter 鍵／登入；停用；規則第一條命中；沒命中問人。

## 任務 3：ComputerService 的向後相容擴充
- `computer/mod.rs`：`pre_authorized: Mutex<HashSet<String>>` 與 `authorize_app_for_system(keys)`／`revoke_...`；`authorize_any` 先查它。
- task-local `PRE_CONFIRMED`；`confirm_destructive` 為真時直接 Ok；匯出 `with_pre_confirmed(fut)`。
- `element_label(app, id) -> Option<(role,title)>`（用 `seen`）、`label_at(app, x, y)`。
- `type_secret(app, element, text)`：gate→activate→focus→確認 `focused_text.secure`→輸入；結果文字固定，審計 detail 只寫「密碼欄位」。
- 測試（`computer/tests.rs` 沿用 FakeBackend）：預先授權不呼叫 approver；PRE_CONFIRMED 不呼叫 destructive；`type_secret` 非密碼欄位拒絕且不輸入；成功時回傳與審計不含密碼。

## 任務 4：無人偵測介面與各平台實作
- `systems/desktop/presence.rs`：`trait InputMonitor { idle_secs, screen_locked }`、`NullMonitor`（回 None）、`FakeMonitor`（測試）、純函式 `user_interfered(idle, since_own_action, tolerance)`、`defer_decision(locked, idle, needed, waited, max)`。
- 各平台實作放 `computer/macos.rs`（CGEventSource 閒置秒數＋CGSession 鎖定字典）、`computer/windows.rs`（GetLastInputInfo＋OpenInputDesktop）、`computer/linux.rs`（x11rb screensaver idle；Wayland 回 None）。
- 測試：兩個純函式的邊界；各平台實作只做編譯檢查（`cargo check --target`）加 macOS 本機呼叫不 panic。

## 任務 5：DesktopRunner 與工具
- `systems/desktop/tools.rs`：`tool_defs()`（`sys_app_*` 的 JSON schema，帶 `system`、`intent`）。
- `systems/desktop/runner.rs`：`DesktopRunner { computer: Arc<ComputerService>, monitor: Arc<dyn InputMonitor>, approvals, audit pool, emitter, 暫停狀態 }`；`call(conn, tool, args)`：停用／暫停檢查 → 衝突偵測 → 取標籤 → `decide_action` → Ask 走 Approvals → `with_pre_confirmed` 執行 → 審計 → 記 `last_own_input_at`；`sys_app_login`、`sys_app_report_login_needed`；`resume`、`stop`。
- 目標 App 一律是 `conn.app`；`launch_app` 時先 `open_app`。
- 測試（假後端／監控／approver）：綁定 App 強制；唯讀拒寫；核准通過／拒絕／逾時；Enter 升級；登入不外洩；衝突後暫停與繼續；停止拒絕等待中核准。

## 任務 6：接進 SystemsState 與端點
- `SystemsState` 持有 `DesktopRunner`（`set_computer(...)` 在 `lib.rs` 注入）；`call` 對桌面分派；`ensure_ready`／`stop`／`close`／`set_manual`／`status` 的桌面行為。
- `endpoint.rs`：`tools/list` 依 scope 內連線種類附加桌面工具；`sys_list` 多回 `kind`、`app`；`describe_systems` 對桌面不回 `start_url`。
- `lib.rs`：電腦操作服務建好後注入；`commands/systems.rs` 新增 `systems_resume`。
- 測試：`handle_rpc` 對桌面連線的 `tools/list`、`sys_list`、`tools/call` 路由；網頁連線回歸。

## 任務 7：定時／問題收集／請 AI 操作
- `jobs.rs`：`build_prompt` 依 kind 產生桌面版提示；`run_goal` 對桌面連線先過 `defer_decision` 迴圈（可注入時間與監控以便測試），逾時回失敗＋通知事件；`unattended_warning` 對桌面補說明。
- `ensure_ready` 對桌面連線確保 App 在跑。
- 測試：延後→閒置後執行；超過上限失敗；鎖定延後；網頁連線不受影響。

## 任務 8：前端
- `ConnectionForm.tsx`：類型選擇、App 欄位、`launch_app`；桌面時隱藏網址／網域／上傳資料夾。
- `SystemsView/index.tsx`、`ApprovalCards.tsx`、`AuditLog.tsx`、`AskPanel.tsx`：類型徽章、桌面動作文字、暫停狀態與「繼續」按鈕（`systems_resume`）。
- `src/lib/i18n.ts` 兩個語系補齊；`src/ipc/` 型別更新。
- 測試：表單切換、徽章、核准卡文字、暫停按鈕；locale 漂移測試。

## 任務 9：收尾
- 全套驗證（Rust workspace、tsc、vitest、lint）；寫 CHANGELOG 使用者看得懂的段落（不打 tag）。
- 回報人工驗收清單（三平台）給使用者。
