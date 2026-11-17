//! 系統連線：讓 AI 自主操作外部網頁系統。見
//! docs/superpowers/specs/2026-11-17-system-connections-design.md。

pub mod approvals;
pub mod audit;
pub mod browser;
pub mod cdp;
pub mod endpoint;
pub mod gate;
pub mod jobs;
pub mod proxy;
pub mod recipes;
pub mod skill;
pub mod state;
pub mod types;

#[cfg(test)]
pub mod testsite;
