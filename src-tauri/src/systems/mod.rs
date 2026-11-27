//! 系統連線：讓 AI 自主操作外部網頁系統。見
//! docs/superpowers/specs/2026-11-17-system-connections-design.md。

// 閘門、CDP、瀏覽器、Playwright 代理、核准等待、型別搬到共用的 aiterm-systems（server 也用）；這裡 re-export，原本的路徑不變。
pub use aiterm_systems::{approvals, browser, cdp, gate, proxy, testsite, types};
pub mod audit;
pub mod desktop;
pub mod endpoint;
pub mod jobs;
pub mod login;
pub mod recipes;
pub mod skill;
pub mod upload;
pub mod state;

