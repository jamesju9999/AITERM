//! 系統連線核心：寫入閘門（`gate`）、CDP 攔截（`cdp`）、啟動 Chrome（`browser`）、代理 Playwright MCP（`proxy`）、
//! 核准等待（`approvals`）與審計（`audit`）。桌面版與 server 共用；與 App 相關的部分（設定檔、鑰匙圈、配方、定時任務）留在各自的程式裡。
//! 見 `docs/superpowers/specs/2026-11-17-system-connections-design.md`。

pub mod approvals;
pub mod audit;
pub mod browser;
pub mod cdp;
pub mod gate;
pub mod launch;
pub mod proxy;
pub mod testsite;
pub mod types;

/// Playwright MCP 的版本（釘死；`browser_*` 工具的名稱與行為跟版本走）。
pub const PLAYWRIGHT_MCP: &str = "@playwright/mcp@0.0.83";
